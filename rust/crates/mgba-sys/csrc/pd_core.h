/* A flat C API over libmgba for the Rust app (crates/mgba-sys/src/lib.rs binds it). The same core setup,
 * save handling and reads as the Android app's JNI bridge (app/src/main/cpp/pokedaisy_jni.c) - keep them in
 * step - but per instance instead of one global core, and with no JNI. */
#ifndef PD_CORE_H
#define PD_CORE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct PdCore PdCore;

#define PD_SAMPLE_RATE 48000

enum { PD_PLATFORM_NONE = 0, PD_PLATFORM_GBA = 1, PD_PLATFORM_GB = 2 };

/* Loads `rom` (UTF-8 path) with its battery save at `save` (created if missing; NULL = none). NULL on failure. */
PdCore* pd_core_open(const char* rom, const char* save);
void pd_core_close(PdCore* c);

int pd_core_platform(const PdCore* c);
unsigned pd_core_width(const PdCore* c);
unsigned pd_core_height(const PdCore* c);
/* The frame: width * height pixels, 4 bytes each, R G B X (the 4th byte is junk, not alpha). */
const uint8_t* pd_core_pixels(const PdCore* c);

void pd_core_set_keys(PdCore* c, uint32_t mask);
void pd_core_run_frame(PdCore* c);
/* Interleaved stereo samples from the last frame, at most `cap` shorts; returns how many were copied. */
size_t pd_core_read_audio(PdCore* c, int16_t* out, size_t cap);
/* Runs silent frames (no keys) while the game is still writing its save, at most `max_frames`. */
int pd_core_finish_save(PdCore* c, int max_frames);

bool pd_core_save_state(PdCore* c, const char* path);
bool pd_core_load_state(PdCore* c, const char* path);
/* 1 = the save inside the state at `path` equals the save file, 0 = differs, -1 = no save in it / unreadable. */
int pd_core_state_matches_save(PdCore* c, const char* path);

/* `len` bytes from the bus at `addr` (ROM, EWRAM, IWRAM...), alignment-safe. False if there is no core. */
bool pd_core_read(PdCore* c, uint32_t addr, uint8_t* dst, size_t len);

/* Calls the game's Thumb function `fn` (r0-r3 = args) between frames, the CPU put back after; r0 = what it
 * returned. False on a Game Boy, or if it never returned. */
bool pd_core_call(PdCore* c, uint32_t fn, const uint32_t args[4], uint32_t* r0);
/* Writes `len` bytes to EWRAM / IWRAM at `addr` (anything else is refused), between frames. GBA only. */
bool pd_core_write(PdCore* c, uint32_t addr, const uint8_t* src, size_t len);
/* mGBA's mixer switch for sound channel `id` (GBA: 0-3 the PSG, 4-5 DirectSound A / B): output only. */
void pd_core_enable_audio_channel(PdCore* c, int id, bool enable);
/* A save state in memory (no save data), read off the core without changing it; *out is malloc'd (pd_core_free). */
bool pd_core_state_to_memory(PdCore* c, uint8_t** out, size_t* len);
bool pd_core_state_from_memory(PdCore* c, const uint8_t* data, size_t len);
void pd_core_free(void* p);
/* Parks the main loop on the Thumb `b .` at `spin`, the VBlank interrupt (and so the sound engine) running on. */
bool pd_core_park(PdCore* c, uint32_t spin);

#ifdef __cplusplus
}
#endif
#endif
