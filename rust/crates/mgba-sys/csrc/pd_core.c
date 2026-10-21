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

#define PD_AUDIO_SCRATCH 8192   /* per channel */

struct PdCore {
    struct mCore* core;
    color_t* video;
    unsigned vw, vh;
    struct VFile* saveVf;
    bool saveReadOnly;
    int16_t audio[PD_AUDIO_SCRATCH * 2];
    size_t audioAvail;
};

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
    /* Unloads the ROM and flushes the save. The core owns the save's VFile from loadSave on and closes it here
     * (GBADestroy / GBDestroy: savedata.realVf, sramRealVf): closing it again was a double free. */
    if (c->core) c->core->deinit(c->core);
    else if (c->saveVf) c->saveVf->close(c->saveVf);
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

void pd_core_run_frame(PdCore* c) {
    if (!c || !c->core) return;
    c->core->runFrame(c->core);
    drainAudio(c);
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
bool pd_core_call(PdCore* c, uint32_t fn, const uint32_t args[4], uint32_t* r0) {
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
bool pd_core_park(PdCore* c, uint32_t spin) {
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

bool pd_core_state_from_memory(PdCore* c, const uint8_t* data, size_t len) {
    if (!c || !c->core || !data) return false;
    struct VFile* vf = VFileFromConstMemory(data, len);
    if (!vf) return false;
    bool ok = mCoreLoadStateNamed(c->core, vf, SAVESTATE_RTC);
    vf->close(vf);
    return ok;
}

void pd_core_free(void* p) { free(p); }
