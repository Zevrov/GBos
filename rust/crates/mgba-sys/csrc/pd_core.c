#include "pd_core.h"

#include <fcntl.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <mgba/core/blip_buf.h>
#include <mgba/core/config.h>
#include <mgba/core/core.h>
#include <mgba/core/log.h>
#include <mgba/core/serialize.h>
#include <mgba-util/vfs.h>
#include <mgba/internal/arm/arm.h>
#include <mgba/internal/arm/isa-inlines.h>
#include <mgba/internal/gb/gb.h>
#include <mgba/internal/gba/gba.h>

#include "pk_wide.h"

#ifdef _WIN32
#include <windows.h>
#endif

#define PD_AUDIO_SCRATCH 8192   /* per channel */

/* The area-name banner's corner of the frame: pk_wide.c's POPUP_ROWS x POPUP_COLS tiles. */
#define BANNER_W (14 * 8)
#define BANNER_H (5 * 8)

struct PdCore {
    struct mCore* core;
    color_t* video;
    unsigned vw, vh;
    struct VFile* saveVf;
    bool saveReadOnly;
    int16_t audio[PD_AUDIO_SCRATCH * 2];
    size_t audioAvail;
    /* EXPAND (pk_wide.c): the window around the frame with the rebuilt scene, wideW x wideH (0 = not drawn: the
     * screen shows `video`). pk_wide.c keeps one game's state, so one core at a time uses it (the player's). */
    uint32_t* wide;
    int wideW, wideH;
    uint32_t wideCrc;          /* the game EXPAND draws (its ROM's CRC32; 0 = off) */
    int viewX, viewY, viewW, viewH;
    bool wideSkipNext;         /* the next frame won't be shown: only noted (pk_wide_skip) */
    bool wideSkipped;          /* the last frame was only noted: wide / wideW still hold the one before */
    unsigned long wideFault;   /* the fault that switched EXPAND off (pd_guard), 0 = none */
    bool wideTestFault;        /* tests: EXPAND's next frame faults */
    /* The area-name banner the games slide in at the frame's top-left: moved to (bannerX, bannerY) of the window
     * (its pixels; -1 = left where the game puts it), with the map pk_wide.c rebuilds under the frame (`centre`)
     * where it was. */
    int bannerX, bannerY;
    uint32_t* centre;
    uint8_t bannerMask[BANNER_H][BANNER_W];   /* the banner's pixels on the last frame (bannerFind) */
    bool bannerMoved;                          /* the last frame's banner was moved */
    /* A helper core (fast-forward's music and shadow cores) runs under pd_guard: a fault in it leaves the core
     * dead - every call a no-op from then on, never torn down (its state is whatever the fault left) - and the
     * player's game goes on. */
    bool guarded;
    unsigned long dead;
};

/* pk_wide.c keeps one game's state (and hooks one renderer): the one core that has EXPAND on. Another core asking
 * while it does is refused; closing it, or switching it off, lets the next one have it. */
static PdCore* volatile wideOwner;

/* --- the fault guard -------------------------------------------------------------------------------------
 * What pk_wide.c's POSIX guard does on Android (sigsetjmp / sigaction), for Windows: a vectored exception handler
 * that acts only while this thread is inside pd_guard, and jumps back out of it (GCC's __builtin_longjmp, as the
 * signal handler's siglongjmp). A fault anywhere else goes on to Windows as if this weren't here. Elsewhere
 * (Linux, macOS) pd_guard just calls through for now. */

typedef void (*PdGuarded)(void* arg);

#ifdef _WIN32
static __thread void* guardBuf[5];          /* __builtin_setjmp's buffer: five words */
static __thread volatile int guardOn;
static __thread volatile unsigned long guardCode;
static volatile LONG guardInstalled;

static LONG CALLBACK guardHandler(EXCEPTION_POINTERS* e) {
    if (!guardOn) return EXCEPTION_CONTINUE_SEARCH;
    DWORD code = e->ExceptionRecord->ExceptionCode;
    switch (code) {
    case EXCEPTION_ACCESS_VIOLATION:
    case EXCEPTION_ARRAY_BOUNDS_EXCEEDED:
    case EXCEPTION_DATATYPE_MISALIGNMENT:
    case EXCEPTION_ILLEGAL_INSTRUCTION:
    case EXCEPTION_IN_PAGE_ERROR:
    case EXCEPTION_INT_DIVIDE_BY_ZERO:
    case EXCEPTION_PRIV_INSTRUCTION:
        break;
    default:
        return EXCEPTION_CONTINUE_SEARCH;   /* not a crash (a debugger's breakpoint, a C++ throw...) */
    }
    guardOn = 0;
    guardCode = code;
    __builtin_longjmp(guardBuf, 1);
}

/* Runs fn(arg); 0 if it returned, else the fault (a Windows exception code) that cut it short. */
static unsigned long __attribute__((noinline)) pd_guard(PdGuarded fn, void* arg) {
    if (!guardInstalled && !InterlockedExchange(&guardInstalled, 1)) AddVectoredExceptionHandler(1, guardHandler);
    if (__builtin_setjmp(guardBuf)) return guardCode ? guardCode : 1;
    guardCode = 0;
    guardOn = 1;
    fn(arg);
    guardOn = 0;
    return 0;
}
#else
static unsigned long pd_guard(PdGuarded fn, void* arg) {
    fn(arg);
    return 0;
}
#endif

static void selftestFault(void* arg) {
    (void) arg;
    *(volatile int*) (uintptr_t) 0x10 = 1;   /* an access violation, on purpose */
}

unsigned long pd_core_guard_selftest(void) {
    return pd_guard(selftestFault, NULL);
}

static bool isGba(const struct mCore* core) { return core && core->platform(core) == mPLATFORM_GBA; }

/* Red / Blue / Yellow are SGB-enhanced: no Super Game Boy border, so every GB game keeps its 160x144. */
static void gbConfig(struct mCore* core) {
    if (core->platform(core) != mPLATFORM_GB) return;
    mCoreConfigSetIntValue(&core->config, "sgb.borders", 0);
    core->reloadConfigOption(core, "sgb.borders", &core->config);
}

/* A cart clock the save brought back broken (dead battery flag, 12-hour on a Pokemon game): a fresh 24-hour one,
 * so Ruby / Sapphire / Emerald don't say "The internal battery has run dry". See pkHealRtc in pokedaisy_jni.c. */
static void healRtc(struct mCore* core) {
    if (!isGba(core)) return;
    struct GBA* gba = (struct GBA*) core->board;
    struct GBARTC* rtc = &gba->memory.hw.rtc;
    if (!(gba->memory.hw.devices & HW_RTC)) return;
    const char* code = gba->memory.rom && gba->memory.romSize > 0xB0 ? (const char*) gba->memory.rom + 0xAC : "";
    bool pokemon = !strncmp(code, "AXV", 3) || !strncmp(code, "AXP", 3) || !strncmp(code, "BPE", 3);
    if (!RTCControlIsPoweroff(rtc->control) && (RTCControlIsHour24(rtc->control) || !pokemon)) return;
    rtc->control = RTCControlFillHour24(0);
    rtc->offset = 0;
}

/* mGBA logs every DMA and BIOS call by default: only its warnings and errors, to stderr. */
static void quietLog(struct mLogger* logger, int category, enum mLogLevel level, const char* format, va_list args) {
    (void) logger;
    if (!(level & (mLOG_ERROR | mLOG_WARN | mLOG_FATAL))) return;
    fprintf(stderr, "mGBA %s: ", mLogCategoryName(category));
    vfprintf(stderr, format, args);
    fputc('\n', stderr);
}

static struct mLogger quietLogger = { .log = quietLog, .filter = NULL };

PdCore* pd_core_open(const char* romPath, const char* savePath) {
    mLogSetDefaultLogger(&quietLogger);
    struct VFile* rom = VFileOpen(romPath, O_RDONLY);
    if (!rom) return NULL;
    struct mCore* core = mCoreFindVF(rom);
    if (!core) { rom->close(rom); return NULL; }
    if (!core->init(core)) { rom->close(rom); core->deinit(core); return NULL; }
    mCoreInitConfig(core, NULL);
    gbConfig(core);

    PdCore* c = calloc(1, sizeof(PdCore));
    if (!c) { rom->close(rom); core->deinit(core); return NULL; }
    c->bannerX = c->bannerY = -1;
    core->desiredVideoDimensions(core, &c->vw, &c->vh);
    c->video = calloc((size_t) c->vw * c->vh, sizeof(color_t));
    core->setVideoBuffer(core, c->video, c->vw);
    core->setAudioBufferSize(core, 2048);
    blip_set_rates(core->getAudioChannel(core, 0), core->frequency(core), PD_SAMPLE_RATE);
    blip_set_rates(core->getAudioChannel(core, 1), core->frequency(core), PD_SAMPLE_RATE);

    if (!core->loadROM(core, rom)) {
        rom->close(rom);
        core->deinit(core);
        free(c->video);
        free(c);
        return NULL;
    }
    if (savePath) {
        /* O_CREAT: a fresh game starts with an empty save file of its own; read-only if it can't be written. */
        c->saveVf = VFileOpen(savePath, O_RDWR | O_CREAT);
        if (!c->saveVf) {
            c->saveVf = VFileOpen(savePath, O_RDONLY);
            c->saveReadOnly = c->saveVf != NULL;
        }
        if (c->saveVf) core->loadSave(core, c->saveVf);
    }
    core->reset(core);
    healRtc(core);
    c->core = core;
    return c;
}

void pd_core_close(PdCore* c) {
    if (!c) return;
    /* A core that faulted is left as it is: tearing down whatever state the fault left could fault again. */
    if (c->dead) {
        if (wideOwner == c) wideOwner = NULL;
        return;
    }
    /* Unloads the ROM and flushes the save. The core owns the save's VFile from loadSave on and closes it here
     * (GBADestroy / GBDestroy: savedata.realVf, sramRealVf): closing it again was a double free. */
    if (wideOwner == c) {
        pk_wide_configure(0);
        wideOwner = NULL;
    }
    if (c->core) c->core->deinit(c->core);
    else if (c->saveVf) c->saveVf->close(c->saveVf);
    free(c->wide);
    free(c->centre);
    free(c->video);
    free(c);
}

int pd_core_platform(const PdCore* c) {
    if (!c || !c->core) return PD_PLATFORM_NONE;
    switch (c->core->platform(c->core)) {
    case mPLATFORM_GBA: return PD_PLATFORM_GBA;
    case mPLATFORM_GB: return PD_PLATFORM_GB;
    default: return PD_PLATFORM_NONE;
    }
}

unsigned pd_core_width(const PdCore* c) { return c ? c->vw : 0; }
unsigned pd_core_height(const PdCore* c) { return c ? c->vh : 0; }
const uint8_t* pd_core_pixels(const PdCore* c) { return c ? (const uint8_t*) c->video : NULL; }

void pd_core_set_keys(PdCore* c, uint32_t mask) { if (c && c->core) c->core->setKeys(c->core, mask); }

static void drainAudio(PdCore* c) {
    struct blip_t* left = c->core->getAudioChannel(c->core, 0);
    struct blip_t* right = c->core->getAudioChannel(c->core, 1);
    int avail = blip_samples_avail(left);
    if (avail > PD_AUDIO_SCRATCH) avail = PD_AUDIO_SCRATCH;
    if (avail > 0) {
        blip_read_samples(left, c->audio, avail, 1);
        blip_read_samples(right, c->audio + 1, avail, 1);
        c->audioAvail = (size_t) avail * 2;
    } else {
        c->audioAvail = 0;
    }
}

struct WideJob {
    PdCore* c;
    struct PkWideView v;
    bool skip;
    uint32_t* centre;   /* the map under the frame too (pk_wide_view's `centre`), or NULL */
};

static void selftestFault(void* arg);

static void wideJob(void* arg) {
    struct WideJob* j = arg;
    if (j->c->wideTestFault) {
        j->c->wideTestFault = false;
        selftestFault(NULL);
    }
    if (j->skip) pk_wide_skip(j->c->core, (const uint32_t*) j->c->video);
    else pk_wide_view(j->c->core, (const uint32_t*) j->c->video, j->c->wide, j->v, j->centre);
}

static void runFrameInner(PdCore* c);

/* The area-name banner's corner of the frame (pk_wide.c's POPUP_ROWS x POPUP_COLS tiles of BG0), and how many of
 * its pixels must show for it to be up. */
#define BANNER_MIN 64

/* mGBA's 32-bit colour for a BGR555 one (mColorFrom555, pk_wide.c's color()): its top 5 bits per channel. */
static inline uint32_t rgb555(uint16_t c) {
    return (uint32_t) (c & 0x1F) << 3 | (uint32_t) ((c >> 5) & 0x1F) << 11 | (uint32_t) ((c >> 10) & 0x1F) << 19;
}

/* BG0's palette index at the frame's (x, y) with vertical scroll `vofs`, as the PPU reads its tilemap and tiles
 * (a 16-colour 256 x 256 text background, as the banner's is); 0 = transparent. Its colour in *col. */
static int bg0At(const uint8_t* vram, const uint16_t* pal, uint32_t chr, uint32_t scr, int hofs, int vofs, int x,
                 int y, uint32_t* col) {
    int px = (x + hofs) & 255, py = (y + vofs) & 255;
    uint32_t e = (uint32_t) (vram[scr + 2 * ((py >> 3) * 32 + (px >> 3))] | vram[scr + 2 * ((py >> 3) * 32 + (px >> 3)) + 1] << 8);
    int ix = px & 7, iy = py & 7;
    if (e & 0x400) ix = 7 - ix;
    if (e & 0x800) iy = 7 - iy;
    uint32_t at = chr + (e & 0x3FF) * 32 + (uint32_t) iy * 4 + (uint32_t) (ix >> 1);
    if (at >= 0x10000) return 0;
    int idx = ix & 1 ? vram[at] >> 4 : vram[at] & 15;
    if (idx) *col = rgb555(pal[(e >> 12) * 16 + idx]);
    return idx;
}

/* The banner's pixels on the frame just drawn, into c->bannerMask: where BG0 is solid in the top-left corner and the
 * frame shows its colour. That colour check also finds the scroll it was drawn with: it slides in by BG0's vertical
 * scroll, and the registers after the frame may be the next one's already. (mGBA's layer tags on the pixels would
 * say it, but blending - the overworld's - strips them.) Returns how many pixels. */
static int bannerFind(PdCore* c) {
    struct GBA* gba = (struct GBA*) c->core->board;
    const uint16_t* io = gba->memory.io;
    uint16_t cnt = io[0x08 >> 1];
    if (!(io[0] & 0x100) || (cnt & 0x80) || (cnt >> 14)) return 0;   /* BG0 off, 256 colours, not 256 x 256 */
    const uint8_t* vram = (const uint8_t*) gba->video.vram;
    const uint16_t* pal = gba->video.palette;
    uint32_t chr = ((cnt >> 2) & 3) * 0x4000u, scr = ((cnt >> 8) & 31) * 0x800u;
    int hofs = io[0x10 >> 1] & 0x1FF, vofs = io[0x12 >> 1] & 0x1FF;
    const uint32_t* f = (const uint32_t*) c->video;
    uint32_t col;
    /* Nothing solid there, at this scroll or a step either way: no banner (the usual case, so it stays cheap). */
    int solid = 0;
    for (int y = 0; y < BANNER_H && solid < BANNER_MIN; y += 2)
        for (int x = 0; x < BANNER_W; x += 2) solid += bg0At(vram, pal, chr, scr, hofs, vofs, x, y, &col) != 0;
    if (solid * 4 < BANNER_MIN / 2) return 0;
    int best = 0, bestDy = 0;
    for (int dy = -12; dy <= 12; dy++) {
        int n = 0, bad = 0;
        for (int y = 0; y < BANNER_H; y++)
            for (int x = 0; x < BANNER_W; x++)
                if (bg0At(vram, pal, chr, scr, hofs, vofs + dy, x, y, &col)) {
                    if (((f[y * PK_WIDE_W + x] ^ col) & 0xF8F8F8) == 0) n++;
                    else bad++;
                }
        if (n > best && bad * 8 < n) {
            best = n;
            bestDy = dy;
        }
    }
    if (best < BANNER_MIN) return 0;
    /* The banner fits the corner; a window that reaches past it (a menu's box, Seaglass's clock) isn't one. */
    int past = 0;
    for (int y = 0; y < BANNER_H + 16; y++)
        for (int x = 0; x < BANNER_W + 32; x++)
            if ((x >= BANNER_W || y >= BANNER_H) && bg0At(vram, pal, chr, scr, hofs, vofs + bestDy, x, y, &col) &&
                ((f[y * PK_WIDE_W + x] ^ col) & 0xF8F8F8) == 0)
                past++;
    if (past >= 16) return 0;
    for (int y = 0; y < BANNER_H; y++)
        for (int x = 0; x < BANNER_W; x++)
            c->bannerMask[y][x] = bg0At(vram, pal, chr, scr, hofs, vofs + bestDy, x, y, &col) &&
                ((f[y * PK_WIDE_W + x] ^ col) & 0xF8F8F8) == 0;
    return best;
}

/* After pk_wide_view drew the window with `centre`: the banner's pixels go where the window's top-left is
 * (bannerX, bannerY, keeping their place in the corner, so it still slides in), and the map under them shows
 * where the frame had them. Only on the overworld (a map was rebuilt, so `centre` holds it). */
static bool moveBanner(PdCore* c, struct PkWideView v) {
    int scene = pk_wide_scene();
    if (scene != PK_WIDE_SCENE_FIELD && scene != PK_WIDE_SCENE_TEXT) return false;
    const uint32_t* f = (const uint32_t*) c->video;
    uint32_t* out = c->wide;
    for (int y = 0; y < BANNER_H; y++)
        for (int x = 0; x < BANNER_W; x++) {
            int i = x - v.x, j = y - v.y;
            if (c->bannerMask[y][x] && i >= 0 && i < v.w && j >= 0 && j < v.h)
                out[j * v.w + i] = c->centre[y * PK_WIDE_W + x];
        }
    for (int y = 0; y < BANNER_H; y++)
        for (int x = 0; x < BANNER_W; x++) {
            uint32_t p = f[y * PK_WIDE_W + x];
            int i = c->bannerX + x, j = c->bannerY + y;
            if (c->bannerMask[y][x] && i >= 0 && i < v.w && j >= 0 && j < v.h) out[j * v.w + i] = p;
        }
    return true;
}

static void runFrameJob(void* arg) {
    runFrameInner(arg);
}

void pd_core_run_frame(PdCore* c) {
    if (!c || !c->core || c->dead) return;
    if (c->guarded) {
        unsigned long f = pd_guard(runFrameJob, c);
        if (f) c->dead = f;
        return;
    }
    runFrameInner(c);
}

static void runFrameInner(PdCore* c) {
    c->core->runFrame(c->core);
    drainAudio(c);
    /* EXPAND: only with a game it knows and a window set (pkRunFrame in pokedaisy_jni.c). */
    struct PkWideView v = { c->viewX, c->viewY, c->viewW, c->viewH };
    bool on = c->wide && c->wideCrc && v.w > 0 && v.h > 0 && v.w <= PK_WIDE_MAX_VIEW_W && v.h <= PK_WIDE_MAX_VIEW_H &&
        sizeof(color_t) == 4 && c->vw == PK_WIDE_W && c->vh == PK_WIDE_H;
    c->wideSkipped = false;
    c->bannerMoved = false;
    if (on) {
        /* A fault in pk_wide.c (a bad table, a game state it didn't expect) switches EXPAND off for this game and
         * the frame goes on plain - instead of taking the app down. */
        /* The banner moves only when the window reaches past the frame's top or left (else it's at the corner
         * already), and the map under it is drawn only while it's up. */
        bool banner = !c->wideSkipNext && c->bannerX >= 0 && (v.x < 0 || v.y < 0) && bannerFind(c) > 0;
        if (banner && !c->centre) c->centre = calloc((size_t) PK_WIDE_W * PK_WIDE_H, sizeof(uint32_t));
        struct WideJob job = { c, v, c->wideSkipNext, banner ? c->centre : NULL };
        unsigned long f = pd_guard(wideJob, &job);
        if (f || pk_wide_fault()) {
            c->wideFault = f ? f : (unsigned long) pk_wide_fault();
            pk_wide_configure(0);
            wideOwner = NULL;
            c->wideCrc = 0;
            c->wideW = c->wideH = 0;
        } else if (job.skip) {
            c->wideSkipped = true;
        } else {
            c->wideW = v.w;
            c->wideH = v.h;
            if (job.centre) c->bannerMoved = moveBanner(c, v);
        }
    } else {
        c->wideW = c->wideH = 0;
    }
}

bool pd_core_wide_knows(uint32_t crc) { return pk_wide_knows(crc) != 0; }

bool pd_core_wide_configure(PdCore* c, uint32_t crc) {
    if (!c || !c->core) return false;
    uint32_t want = isGba(c->core) ? crc : 0;
    if (want && wideOwner && wideOwner != c) return false;
    if (want == c->wideCrc) return want != 0;
    if (want && !c->wide) c->wide = calloc((size_t) PK_WIDE_MAX_VIEW_W * PK_WIDE_MAX_VIEW_H, sizeof(uint32_t));
    c->wideCrc = want && c->wide && pk_wide_configure(want) ? want : 0;
    if (c->wideCrc) {
        wideOwner = c;
    } else {
        if (wideOwner == c) {
            pk_wide_configure(0);
            wideOwner = NULL;
        }
        c->wideW = c->wideH = 0;
    }
    return c->wideCrc != 0;
}

void pd_core_wide_set_view(PdCore* c, int x, int y, int w, int h) {
    if (!c) return;
    c->viewX = x;
    c->viewY = y;
    c->viewW = w < 0 ? 0 : w > PK_WIDE_MAX_VIEW_W ? PK_WIDE_MAX_VIEW_W : w;
    c->viewH = h < 0 ? 0 : h > PK_WIDE_MAX_VIEW_H ? PK_WIDE_MAX_VIEW_H : h;
}

void pd_core_wide_set_draw(PdCore* c, bool draw) { if (c) c->wideSkipNext = !draw; }

const uint8_t* pd_core_wide_banner(const PdCore* c) {
    return c && c->bannerMoved ? &c->bannerMask[0][0] : NULL;
}

void pd_core_wide_set_banner(PdCore* c, int x, int y) {
    if (!c) return;
    c->bannerX = x < 0 || y < 0 ? -1 : x;
    c->bannerY = x < 0 || y < 0 ? -1 : y;
}

const uint8_t* pd_core_wide_pixels(const PdCore* c, int* w, int* h) {
    if (!c || !c->wide || c->wideW <= 0 || c->wideSkipped) { *w = *h = 0; return NULL; }
    *w = c->wideW;
    *h = c->wideH;
    return (const uint8_t*) c->wide;
}

int pd_core_wide_scene(const PdCore* c) { return c && c->wideW ? pk_wide_scene() : PK_WIDE_SCENE_OFF; }

unsigned long pd_core_wide_fault(const PdCore* c) { return c ? c->wideFault : 0; }

void pd_core_set_guarded(PdCore* c, bool guarded) { if (c) c->guarded = guarded; }

unsigned long pd_core_dead(const PdCore* c) { return c ? c->dead : 0; }

static void testFaultJob(void* arg) {
    (void) arg;
    selftestFault(NULL);
}

/* Tests: EXPAND's next frame faults, inside pk_wide's guard. */
void pd_core_wide_test_fault(PdCore* c) { if (c) c->wideTestFault = true; }

/* Tests: a fault inside this core's guarded work, as a real one would leave it. */
void pd_core_test_fault(PdCore* c) {
    if (!c) return;
    unsigned long f = pd_guard(testFaultJob, c);
    if (f && c->guarded) c->dead = f;
}

size_t pd_core_read_audio(PdCore* c, int16_t* out, size_t cap) {
    if (!c || !c->audioAvail) return 0;
    size_t n = c->audioAvail < cap ? c->audioAvail : cap;
    memcpy(out, c->audio, n * sizeof(int16_t));
    c->audioAvail = 0;
    return n;
}

static bool saveDirty(struct mCore* core) {
    if (isGba(core)) return ((struct GBA*) core->board)->memory.savedata.dirty != 0;
    if (core->platform(core) == mPLATFORM_GB) return ((struct GB*) core->board)->sramDirty != 0;
    return false;
}

int pd_core_finish_save(PdCore* c, int maxFrames) {
    if (!c || !c->core) return 0;
    int n = 0;
    if (saveDirty(c->core)) c->core->setKeys(c->core, 0);
    while (n < maxFrames && saveDirty(c->core)) {
        c->core->runFrame(c->core);
        drainAudio(c);
        c->audioAvail = 0;
        n++;
    }
    return n;
}

bool pd_core_save_state(PdCore* c, const char* path) {
    if (!c || !c->core) return false;
    struct VFile* vf = VFileOpen(path, O_RDWR | O_CREAT | O_TRUNC);   /* the mpack writer reads back */
    if (!vf) return false;
    bool ok = mCoreSaveStateNamed(c->core, vf, SAVESTATE_SAVEDATA | SAVESTATE_RTC | SAVESTATE_METADATA);
    vf->close(vf);
    return ok;
}

bool pd_core_load_state(PdCore* c, const char* path) {
    if (!c || !c->core) return false;
    struct VFile* vf = VFileOpen(path, O_RDONLY);
    if (!vf) return false;
    bool ok = mCoreLoadStateNamed(c->core, vf, SAVESTATE_SAVEDATA | SAVESTATE_RTC);
    vf->close(vf);
    if (ok) healRtc(c->core);
    if (c->wideCrc) pk_wide_reset();   /* the RAM it kept from the last frame is another moment's */
    return ok;
}

/* The save file holds exactly `data` (missing bytes count as erased flash, 0xFF). */
static bool saveFileEquals(PdCore* c, const uint8_t* data, size_t size) {
    struct VFile* vf = c->saveVf;
    if (!vf || c->saveReadOnly) return true;
    uint8_t buf[4096];
    size_t off = 0;
    vf->seek(vf, 0, SEEK_SET);
    while (off < size) {
        size_t want = size - off < sizeof(buf) ? size - off : sizeof(buf);
        ssize_t n = vf->read(vf, buf, want);
        if (n <= 0) break;
        if (memcmp(buf, data + off, (size_t) n) != 0) return false;
        off += (size_t) n;
    }
    for (; off < size; ++off) if (data[off] != 0xFF) return false;
    return true;
}

int pd_core_state_matches_save(PdCore* c, const char* path) {
    if (!c || !c->core) return -1;
    struct VFile* vf = VFileOpen(path, O_RDONLY);
    if (!vf) return -1;
    int result = -1;
    struct mStateExtdata extdata;
    mStateExtdataInit(&extdata);
    if (mCoreExtractExtdata(c->core, vf, &extdata)) {
        struct mStateExtdataItem item;
        if (mStateExtdataGet(&extdata, EXTDATA_SAVEDATA, &item) && item.data && item.size > 0) {
            result = saveFileEquals(c, item.data, (size_t) item.size) ? 1 : 0;
        }
    }
    mStateExtdataDeinit(&extdata);
    vf->close(vf);
    return result;
}

bool pd_core_read(PdCore* c, uint32_t addr, uint8_t* dst, size_t len) {
    if (!c || !c->core) return false;
    struct mCore* core = c->core;
    size_t i = 0;
    /* Bytes up to a word boundary first: busRead32 rotates an unaligned word like the CPU's LDR. */
    for (; i < len && ((addr + i) & 3); i++) dst[i] = (uint8_t) core->busRead8(core, addr + (uint32_t) i);
    for (; i + 4 <= len; i += 4) {
        uint32_t v = core->busRead32(core, addr + (uint32_t) i);
        dst[i] = (uint8_t) v;
        dst[i + 1] = (uint8_t) (v >> 8);
        dst[i + 2] = (uint8_t) (v >> 16);
        dst[i + 3] = (uint8_t) (v >> 24);
    }
    for (; i < len; i++) dst[i] = (uint8_t) core->busRead8(core, addr + (uint32_t) i);
    return true;
}

/* --- calling the game's own code (the FF music core) ---------------------------------------------------
 * pk_call / pkRenderPark in pokedaisy_jni.c, per instance: keep them in step. */

#define PD_CALL_SENTINEL 0x080000C0u /* an address in the ROM header, never executed */

/* Calls Thumb function `fn` with r0-r3 = `args` and returns once it has, leaving the CPU exactly as it was
 * (registers, pipeline, halt, the fetch region and the idle-loop detector): a frame can end anywhere, and a
 * bare PC hijack clobbered the game's live registers. Runs instruction by instruction with IRQs masked until
 * the call returns to PD_CALL_SENTINEL, then puts everything back. */
struct CallJob {
    PdCore* c;
    uint32_t fn;
    const uint32_t* args;
    uint32_t* r0;
    bool ok;
};

static bool callInner(PdCore* c, uint32_t fn, const uint32_t args[4], uint32_t* r0);

static void callJob(void* arg) {
    struct CallJob* j = arg;
    j->ok = callInner(j->c, j->fn, j->args, j->r0);
}

bool pd_core_call(PdCore* c, uint32_t fn, const uint32_t args[4], uint32_t* r0) {
    if (!c || c->dead) return false;
    if (!c->guarded) return callInner(c, fn, args, r0);
    struct CallJob j = { c, fn, args, r0, false };
    unsigned long f = pd_guard(callJob, &j);
    if (f) c->dead = f;
    return !f && j.ok;
}

static bool callInner(PdCore* c, uint32_t fn, const uint32_t args[4], uint32_t* r0) {
    if (!c || !c->core || !isGba(c->core)) return false;
    struct mCore* core = c->core;
    struct ARMCore* cpu = (struct ARMCore*) core->cpu;
    struct ARMRegisterFile saved = cpu->regs;
    uint32_t prefetch0 = cpu->prefetch[0], prefetch1 = cpu->prefetch[1];
    struct ARMMemory mem = cpu->memory;
    struct GBA* gba = (struct GBA*) core->board;
    int region = gba->memory.activeRegion;
    uint32_t biosPrefetch = gba->memory.biosPrefetch;
    uint32_t idleLoop = gba->idleLoop, lastJump = gba->lastJump;
    bool haltPending = gba->haltPending;
    int idleStep = gba->idleDetectionStep, idleFailures = gba->idleDetectionFailures;
    int32_t cached[16];
    memcpy(cached, gba->cachedRegisters, sizeof(cached));
    enum ExecutionMode exec = cpu->executionMode;
    int halted = cpu->halted;
    enum PrivilegeMode priv = cpu->privilegeMode;
    cpu->halted = 0;
    cpu->cpsr.i = 1;
    /* On the main stack, in System mode: an interrupt's own stack is tiny. */
    if (priv != MODE_SYSTEM && priv != MODE_USER) {
        ARMSetPrivilegeMode(cpu, MODE_SYSTEM);
        cpu->cpsr.priv = MODE_SYSTEM;
    }
    for (int i = 0; i < 4; i++) cpu->gprs[i] = (int32_t) args[i];
    cpu->gprs[ARM_LR] = (int32_t) (PD_CALL_SENTINEL | 1);
    cpu->gprs[ARM_PC] = (int32_t) (fn & ~1u);
    _ARMSetMode(cpu, MODE_THUMB);
    ThumbWritePC(cpu);
    bool ok = false;
    for (int i = 0; i < 2000000; i++) {
        if (cpu->executionMode == MODE_THUMB && (uint32_t) cpu->gprs[ARM_PC] - WORD_SIZE_THUMB == PD_CALL_SENTINEL) {
            ok = true;
            break;
        }
        core->step(core);
    }
    if (r0) *r0 = (uint32_t) cpu->gprs[0];
    cpu->regs = saved;
    cpu->privilegeMode = priv;
    cpu->executionMode = exec == MODE_ARM ? MODE_THUMB : MODE_ARM; /* so _ARMSetMode applies */
    _ARMSetMode(cpu, exec);
    cpu->prefetch[0] = prefetch0;
    cpu->prefetch[1] = prefetch1;
    cpu->memory = mem;
    gba->memory.activeRegion = region;
    gba->memory.biosPrefetch = biosPrefetch;
    gba->idleLoop = idleLoop;
    gba->lastJump = lastJump;
    gba->haltPending = haltPending;
    gba->idleDetectionStep = idleStep;
    gba->idleDetectionFailures = idleFailures;
    memcpy(gba->cachedRegisters, cached, sizeof(cached));
    cpu->halted = halted;
    return ok;
}

/* Parks the main loop on `spin` (a Thumb `b .`) so the booted game can't change the music; the VBlank
 * interrupt keeps the sound engine running. Steps first until the CPU is in the main context with the VBlank
 * IRQ fully enabled (IME, IE bit 0, DISPSTAT bit 3): parked with it off, the music froze. */
struct ParkJob {
    PdCore* c;
    uint32_t spin;
    bool ok;
};

static bool parkInner(PdCore* c, uint32_t spin);

static void parkJob(void* arg) {
    struct ParkJob* j = arg;
    j->ok = parkInner(j->c, j->spin);
}

bool pd_core_park(PdCore* c, uint32_t spin) {
    if (!c || c->dead) return false;
    if (!c->guarded) return parkInner(c, spin);
    struct ParkJob j = { c, spin, false };
    unsigned long f = pd_guard(parkJob, &j);
    if (f) c->dead = f;
    return !f && j.ok;
}

static bool parkInner(PdCore* c, uint32_t spin) {
    if (!c || !c->core || !isGba(c->core)) return false;
    struct mCore* core = c->core;
    struct ARMCore* cpu = (struct ARMCore*) core->cpu;
    for (int i = 0; i < 5000000; i++) {
        if ((cpu->privilegeMode == MODE_SYSTEM || cpu->privilegeMode == MODE_USER) && !cpu->cpsr.i && !cpu->halted &&
            (core->busRead16(core, 0x04000208) & 1) && (core->busRead16(core, 0x04000200) & 1) &&
            (core->busRead16(core, 0x04000004) & 8)) {
            cpu->gprs[ARM_PC] = (int32_t) (spin & ~1u);
            _ARMSetMode(cpu, MODE_THUMB);
            ThumbWritePC(cpu);
            return true;
        }
        core->step(core);
    }
    return false;
}

/* Writes `len` bytes at `addr` - EWRAM / IWRAM only, so nothing else (I/O, VRAM, ROM) can be poked through it.
 * Between frames, like pkWriteBytes in pokedaisy_jni.c. */
bool pd_core_write(PdCore* c, uint32_t addr, const uint8_t* src, size_t len) {
    if (!c || !c->core || !isGba(c->core) || !len) return false;
    bool ram = (addr >= 0x02000000u && addr + len <= 0x02040000u) || (addr >= 0x03000000u && addr + len <= 0x03008000u);
    if (!ram) return false;
    for (size_t i = 0; i < len; i++) c->core->busWrite8(c->core, addr + (uint32_t) i, src[i]);
    return true;
}

/* mGBA's own mixer switch for one sound channel (GBA: 0-3 the PSG's square 1, square 2, wave and noise, 4-5
 * DirectSound A / B). Output only: the game's state, and any save state, never see it. */
void pd_core_enable_audio_channel(PdCore* c, int id, bool enable) {
    if (c && c->core && c->core->enableAudioChannel) c->core->enableAudioChannel(c->core, (size_t) id, enable);
}

/* A save state in memory (no save data in it: the fast-forward shadow core's copy of the live game). Reading
 * the core only: the game it's taken from isn't changed. *out is malloc'd; free it with pd_core_free. */
bool pd_core_state_to_memory(PdCore* c, uint8_t** out, size_t* len) {
    if (!c || !c->core || !out || !len) return false;
    struct VFile* vf = VFileMemChunk(NULL, 0);
    if (!vf) return false;
    bool ok = mCoreSaveStateNamed(c->core, vf, SAVESTATE_RTC);
    if (ok) {
        *len = (size_t) vf->size(vf);
        *out = malloc(*len);
        ok = *out != NULL;
        if (ok) {
            vf->seek(vf, 0, SEEK_SET);
            ok = vf->read(vf, *out, *len) == (ssize_t) *len;
            if (!ok) free(*out);
        }
    }
    vf->close(vf);
    return ok;
}

struct LoadJob {
    PdCore* c;
    const uint8_t* data;
    size_t len;
    bool ok;
};

static bool stateFromMemoryInner(PdCore* c, const uint8_t* data, size_t len);

static void loadJob(void* arg) {
    struct LoadJob* j = arg;
    j->ok = stateFromMemoryInner(j->c, j->data, j->len);
}

bool pd_core_state_from_memory(PdCore* c, const uint8_t* data, size_t len) {
    if (!c || c->dead) return false;
    if (!c->guarded) return stateFromMemoryInner(c, data, len);
    struct LoadJob j = { c, data, len, false };
    unsigned long f = pd_guard(loadJob, &j);
    if (f) c->dead = f;
    return !f && j.ok;
}

static bool stateFromMemoryInner(PdCore* c, const uint8_t* data, size_t len) {
    if (!c || !c->core || !data) return false;
    struct VFile* vf = VFileFromConstMemory(data, len);
    if (!vf) return false;
    bool ok = mCoreLoadStateNamed(c->core, vf, SAVESTATE_RTC);
    vf->close(vf);
    return ok;
}

void pd_core_free(void* p) { free(p); }

/* --- the crash log ----------------------------------------------------------------------------------------
 * A native crash nothing caught (the fault guard only covers EXPAND and the helper cores): one line in the log
 * (the exception, and where: module + offset), and the flag the next launch looks for. Then Windows ends the
 * process as it would have. */
#ifdef _WIN32
static wchar_t crashLog[MAX_PATH * 2];
static wchar_t crashFlag[MAX_PATH * 2];

static LONG WINAPI crashFilter(EXCEPTION_POINTERS* e) {
    FILE* f = _wfopen(crashLog, L"a");
    if (f) {
        void* at = e->ExceptionRecord->ExceptionAddress;
        HMODULE m = NULL;
        char name[MAX_PATH] = "?";
        if (GetModuleHandleExA(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                               (LPCSTR) at, &m)) {
            GetModuleFileNameA(m, name, sizeof(name));
        }
        SYSTEMTIME t;
        GetLocalTime(&t);
        fprintf(f, "%04d-%02d-%02d %02d:%02d:%02d native crash: exception 0x%08lx at %p (%s + 0x%llx)\n\n", t.wYear,
                t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, (unsigned long) e->ExceptionRecord->ExceptionCode, at,
                name, (unsigned long long) ((char*) at - (char*) m));
        fclose(f);
    }
    FILE* flag = _wfopen(crashFlag, L"w");
    if (flag) fclose(flag);
    return EXCEPTION_CONTINUE_SEARCH;
}

void pd_crash_log_install(const char* logUtf8, const char* flagUtf8) {
    MultiByteToWideChar(CP_UTF8, 0, logUtf8, -1, crashLog, MAX_PATH * 2);
    MultiByteToWideChar(CP_UTF8, 0, flagUtf8, -1, crashFlag, MAX_PATH * 2);
    SetUnhandledExceptionFilter(crashFilter);
}
#else
void pd_crash_log_install(const char* logUtf8, const char* flagUtf8) {
    (void) logUtf8;
    (void) flagUtf8;
}
#endif
