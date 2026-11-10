/*
 * render.c: BRYX's build-time cue renderer.
 *
 * build.rs compiles this file with the Electric Pulse engine (audio/engine/,
 * BRYX's fork: see its PROVENANCE.md) into a host tool and runs it once per
 * cue:
 *
 *     render <in.abc> <out.s16> <sample_rate>
 *
 * writes signed 16-bit little-endian mono PCM (the engine's precision),
 * which build.rs encodes to the 8-bit mu-law the game embeds.
 *
 *     render <in.score> <out.qoa|out.f32|out.s16> [sample_rate]
 *
 * renders a studio score (audio/engine/studio.c: float stereo, 48 kHz by
 * default) and writes it as QOA (what the game embeds and decodes, see
 * src/audio/qoa.rs), as raw little-endian float stereo (.f32, for the
 * probes), or as 16-bit stereo (.s16). QOA and .s16 are dithered to 16
 * bit with a fixed-seed TPDF, the only quantization in the studio path.
 *
 * No C runs at game time.
 */
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "audio_engine.h"
#include "studio.h"

#define QOA_IMPLEMENTATION
#include "../vendor/qoa.h"

static int ends_with(const char *s, const char *suffix)
{
    size_t n = strlen(s), m = strlen(suffix);
    return n >= m && !strcmp(s + n - m, suffix);
}

static int write_abc(const char *in, const char *out_path, int rate)
{
    int len = 0;
    int16_t *pcm = audio_engine_render_abc_file_s16(in, rate, &len);
    FILE *out;
    int ok = 1;

    if (!pcm || len <= 0) {
        fprintf(stderr, "render failed: %s\n", in);
        return 1;
    }
    out = fopen(out_path, "wb");
    for (int i = 0; out && ok && i < len; i++) {
        unsigned short v = (unsigned short)pcm[i];
        unsigned char le[2] = { (unsigned char)(v & 0xff), (unsigned char)(v >> 8) };
        ok = fwrite(le, 1, 2, out) == 2;
    }
    if (!out || !ok) {
        fprintf(stderr, "cannot write %s\n", out_path);
        audio_engine_free_s16(pcm);
        if (out) fclose(out);
        return 1;
    }
    fclose(out);
    audio_engine_free_s16(pcm);
    return 0;
}

/* Float stereo to 16 bit with TPDF dither from a fixed seed. */
static short *to_s16(const StRender *r)
{
    size_t n = (size_t)r->frame_count * 2;
    short *s = malloc(n * sizeof *s);
    uint32_t seed = 0x5EED1234u;
    for (size_t i = 0; s && i < n; i++) {
        float d = (st_randf(&seed) + st_randf(&seed)) * 0.5f / 32768.0f;
        float v = r->frames[i] + d;
        long q = lrintf(v * 32767.0f);
        s[i] = (short)(q > 32767 ? 32767 : q < -32768 ? -32768 : q);
    }
    return s;
}

static int write_score(const char *in, const char *out_path, int rate)
{
    char err[512];
    StRender r;
    int rc = 1;
    FILE *out;

    if (st_render_score(in, rate, &r, err, sizeof err)) {
        fprintf(stderr, "render failed: %s\n", err);
        return 1;
    }
    out = fopen(out_path, "wb");
    if (!out) {
        fprintf(stderr, "cannot write %s\n", out_path);
        st_render_free(&r);
        return 1;
    }
    if (ends_with(out_path, ".f32")) {
        rc = fwrite(r.frames, sizeof(float), (size_t)r.frame_count * 2, out) != (size_t)r.frame_count * 2;
    } else {
        short *s16 = to_s16(&r);
        if (s16 && ends_with(out_path, ".qoa")) {
            qoa_desc desc = {0};
            unsigned int len = 0;
            desc.channels = 2;
            desc.samplerate = (unsigned)r.rate;
            desc.samples = (unsigned)r.frame_count;
            void *enc = qoa_encode(s16, &desc, &len);
            rc = !enc || fwrite(enc, 1, len, out) != len;
            free(enc);
        } else if (s16) {
            rc = fwrite(s16, sizeof(short), (size_t)r.frame_count * 2, out) != (size_t)r.frame_count * 2;
        }
        free(s16);
    }
    if (rc) fprintf(stderr, "cannot write %s\n", out_path);
    fclose(out);
    st_render_free(&r);
    return rc;
}

int main(int argc, char **argv)
{
    if (argc < 3 || argc > 4) {
        fprintf(stderr, "usage: %s <in.abc> <out.s16> <sample_rate>\n"
                        "       %s <in.score> <out.qoa|.f32|.s16> [sample_rate]\n", argv[0], argv[0]);
        return 2;
    }
    if (ends_with(argv[1], ".score"))
        return write_score(argv[1], argv[2], argc == 4 ? atoi(argv[3]) : 0);
    if (argc != 4) {
        fprintf(stderr, "an ABC cue needs a sample rate\n");
        return 2;
    }
    return write_abc(argv[1], argv[2], atoi(argv[3]));
}
