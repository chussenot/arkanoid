/*
 * render.c: BRYX's build-time cue renderer.
 *
 * build.rs compiles this file with the Electric Pulse engine (audio/engine/,
 * BRYX's fork: see its PROVENANCE.md) into a host tool and runs it once per
 * cue:
 *
 *     render <in.abc> <out.s16> <sample_rate>
 *
 * Output is signed 16-bit little-endian mono PCM, the engine's internal
 * precision, with nothing around it. build.rs encodes it to the 8-bit
 * mu-law the game embeds and plays through src/audio/; no C runs at game
 * time.
 */
#include <stdio.h>
#include <stdlib.h>

#include "audio_engine.h"

int main(int argc, char **argv)
{
    int len = 0;
    int16_t *pcm;
    FILE *out;
    int ok = 1;

    if (argc != 4) {
        fprintf(stderr, "usage: %s <in.abc> <out.s16> <sample_rate>\n", argv[0]);
        return 2;
    }
    pcm = audio_engine_render_abc_file_s16(argv[1], atoi(argv[3]), &len);
    if (!pcm || len <= 0) {
        fprintf(stderr, "render failed: %s\n", argv[1]);
        return 1;
    }
    out = fopen(argv[2], "wb");
    for (int i = 0; out && ok && i < len; i++) {
        unsigned short v = (unsigned short)pcm[i];
        unsigned char le[2] = { (unsigned char)(v & 0xff), (unsigned char)(v >> 8) };
        ok = fwrite(le, 1, 2, out) == 2;
    }
    if (!out || !ok) {
        fprintf(stderr, "cannot write %s\n", argv[2]);
        audio_engine_free_s16(pcm);
        if (out) fclose(out);
        return 1;
    }
    fclose(out);
    audio_engine_free_s16(pcm);
    return 0;
}
