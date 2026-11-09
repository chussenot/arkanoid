/*
 * render.c: BRYX's build-time cue renderer.
 *
 * build.rs compiles this file with the vendored Electric Pulse engine
 * (audio/engine/) into a host tool and runs it once per cue:
 *
 *     render <in.abc> <out.u8> <sample_rate>
 *
 * Output is the engine's native format: unsigned 8-bit mono PCM centered
 * at 128, with nothing around it. The game embeds the bytes and plays
 * them through src/audio/; no C runs at game time.
 */
#include <stdio.h>
#include <stdlib.h>

#include "audio_engine.h"

int main(int argc, char **argv)
{
    int len = 0;
    unsigned char *pcm;
    FILE *out;

    if (argc != 4) {
        fprintf(stderr, "usage: %s <in.abc> <out.u8> <sample_rate>\n", argv[0]);
        return 2;
    }
    pcm = audio_engine_render_abc_file(argv[1], atoi(argv[3]), &len, NULL);
    if (!pcm || len <= 0) {
        fprintf(stderr, "render failed: %s\n", argv[1]);
        return 1;
    }
    out = fopen(argv[2], "wb");
    if (!out || fwrite(pcm, 1, (size_t)len, out) != (size_t)len) {
        fprintf(stderr, "cannot write %s\n", argv[2]);
        audio_engine_free_buffer(pcm);
        if (out) fclose(out);
        return 1;
    }
    fclose(out);
    audio_engine_free_buffer(pcm);
    return 0;
}
