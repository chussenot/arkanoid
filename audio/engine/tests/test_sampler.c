/*
 * test_sampler.c: checks for the studio sampler (BRYX addition; see
 * studio.h and studio_sampler.h).
 *
 *   cd audio/engine && cc -std=c99 -O2 -Wall -Wextra -Werror \
 *     -o /tmp/test_sampler tests/test_sampler.c studio_sampler.c \
 *     studio_common.c -lm && /tmp/test_sampler
 *
 * Paths to the sample maps are relative to audio/engine. Exits non-zero
 * when any check fails.
 */
#define _POSIX_C_SOURCE 200809L /* clock_gettime, opendir */

#include <dirent.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include "../studio_sampler.h"

#define PI 3.14159265358979323846
#define SRC_RATE 44100

static int failures;

static void check(int ok, const char *what, const char *detail)
{
    printf("%s  %s%s%s\n", ok ? "ok  " : "FAIL", what, detail ? ": " : "",
           detail ? detail : "");
    if (!ok)
        failures++;
}

/* ------------------------------------------------------------------ */
/* Helpers                                                             */

static StRegion region_template(int lokey, int hikey, int keycenter)
{
    StRegion r;
    memset(&r, 0, sizeof r);
    r.lokey = lokey;
    r.hikey = hikey;
    r.keycenter = keycenter;
    r.lovel = 1;
    r.hivel = 127;
    r.seq_length = 1;
    r.seq_position = 1;
    return r;
}

static float *sine(double hz, double amp, int frames, int rate)
{
    float *x = malloc((size_t)frames * sizeof(float));
    int i;
    for (i = 0; i < frames; i++)
        x[i] = (float)(amp * sin(2.0 * PI * hz * i / rate));
    return x;
}

static StSamplerPatch plain_patch(void)
{
    StSamplerPatch p;
    st_sampler_patch_default(&p);
    return p;
}

/* Renders `frames` of a started voice in ST_BLOCK blocks into fresh
 * zeroed buffers; returns the frame at which render first returned 0
 * (or `frames` if it never did). */
static int render(StSamplerVoice *v, float *l, float *r, int frames)
{
    int at = 0, end = frames;
    memset(l, 0, (size_t)frames * sizeof(float));
    memset(r, 0, (size_t)frames * sizeof(float));
    while (at < frames) {
        int n = frames - at < ST_BLOCK ? frames - at : ST_BLOCK;
        if (!st_sampler_render(v, NULL, l + at, r + at, n) && end == frames)
            end = at + n;
        at += n;
    }
    return end;
}

/* Frequency by rising zero crossings, each placed by linear
 * interpolation, over x[0..n). */
static double zc_freq(const float *x, int n, double rate)
{
    double first = -1.0, last = -1.0;
    int count = 0, i;
    for (i = 1; i < n; i++) {
        if (x[i - 1] < 0.0f && x[i] >= 0.0f) {
            double t = (i - 1) + x[i - 1] / (double)(x[i - 1] - x[i]);
            if (first < 0.0)
                first = t;
            last = t;
            count++;
        }
    }
    if (count < 2)
        return 0.0;
    return (count - 1) * rate / (last - first);
}

/* Frequency by autocorrelation: the best lag between the periods of
 * 0.8x and 1.25x the expected frequency (no octave errors), refined by a
 * parabola through its neighbours. */
static double ac_freq(const float *x, int n, double rate, double expect)
{
    int lo = (int)(rate / (expect * 1.25)), hi = (int)(rate / (expect * 0.8));
    int lag, best = lo, w = n - hi - 1;
    double c[2048], bestc = -1e30;
    if (hi >= 2047 || w <= 0)
        return 0.0;
    for (lag = lo - 1; lag <= hi + 1; lag++) {
        double acc = 0.0, e0 = 0.0, e1 = 0.0;
        int i;
        for (i = 0; i < w; i++) {
            acc += (double)x[i] * x[i + lag];
            e0 += (double)x[i] * x[i];
            e1 += (double)x[i + lag] * x[i + lag];
        }
        c[lag] = acc / sqrt(e0 * e1 + 1e-30);
        if (lag >= lo && lag <= hi && c[lag] > bestc) {
            bestc = c[lag];
            best = lag;
        }
    }
    {
        double a = c[best - 1], b = c[best], d = c[best + 1];
        double den = a - 2.0 * b + d;
        double off = den != 0.0 ? 0.5 * (a - d) / den : 0.0;
        return rate / (best + off);
    }
}

static double cents(double f, double ref)
{
    return 1200.0 * log2(f / ref);
}

static float peak(const float *x, int n)
{
    float p = 0.0f;
    int i;
    for (i = 0; i < n; i++)
        if (fabsf(x[i]) > p)
            p = fabsf(x[i]);
    return p;
}

static double rms(const float *x, int n)
{
    double acc = 0.0;
    int i;
    for (i = 0; i < n; i++)
        acc += (double)x[i] * x[i];
    return sqrt(acc / n);
}

static double now(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec * 1e-9;
}

/* ------------------------------------------------------------------ */
/* 1. Pitch tracking                                                   */

static void test_pitch(void)
{
    const double c4 = 261.63;
    int frames = 3 * SRC_RATE;
    float *x = sine(c4, 0.5, frames, SRC_RATE);
    StSampleSet *set = st_sampleset_new();
    StRegion t = region_template(0, 127, 60);
    StSamplerPatch p = plain_patch();
    static float l[48000], r[48000];
    const int notes[2] = {67, 55};
    const float rates[2] = {48000.0f, 44100.0f};
    int a, b;

    check(st_sampleset_add_region(set, &t, x, frames, SRC_RATE) == 0,
          "pitch: add a sine region", NULL);
    for (a = 0; a < 2; a++) {
        for (b = 0; b < 2; b++) {
            StSamplerVoice v;
            double want = c4 * pow(2.0, (notes[a] - 60) / 12.0);
            int skip = (int)(0.1f * rates[b]), win = (int)(0.5f * rates[b]);
            double got, dc;
            char msg[160];
            st_sampler_start(&v, set, &p, rates[b], (float)notes[a], 127, 0, 1);
            render(&v, l, r, skip + win);
            got = zc_freq(l + skip, win, rates[b]);
            dc = cents(got, want);
            snprintf(msg, sizeof msg,
                     "note %d at %.0f Hz out: %.3f Hz, want %.3f (%+.3f cents)",
                     notes[a], rates[b], got, want, dc);
            check(fabs(dc) <= 3.0, "pitch tracking", msg);
        }
    }
    /* A fractional note and mod->pitch_cents bend by the same amount. */
    {
        StSamplerVoice v;
        StVoiceMod m = {1.0f, 1.0f, 0.0f, 50.0f};
        int k;
        double got, want = c4 * pow(2.0, 1.0 / 12.0);
        char msg[160];
        st_sampler_start(&v, set, &p, 48000.0f, 60.5f, 127, 0, 1);
        memset(l, 0, sizeof l);
        memset(r, 0, sizeof r);
        for (k = 0; k + ST_BLOCK <= 28800; k += ST_BLOCK)
            st_sampler_render(&v, &m, l + k, r + k, ST_BLOCK);
        got = zc_freq(l + 4800, 24000, 48000.0);
        snprintf(msg, sizeof msg, "note 60.5 + 50 cents: %+.3f cents off 61",
                 cents(got, want));
        check(fabs(cents(got, want)) <= 3.0, "pitch bend", msg);
    }
    st_sampleset_free(set);
    free(x);
}

/* ------------------------------------------------------------------ */
/* 2-4. Zone, velocity and round-robin selection, told apart by level   */

static float level_of(const StSampleSet *set, float note, int vel, unsigned rr)
{
    StSamplerVoice v;
    StSamplerPatch p = plain_patch();
    static float l[9600], r[9600];
    p.velocity_curve = 1e-6f; /* ~flat: level shows the region alone */
    if (!st_sampler_start(&v, set, &p, 48000.0f, note, vel, rr, 1))
        return -1.0f;
    render(&v, l, r, 9600);
    return peak(l + 2400, 7200);
}

static void test_selection(void)
{
    int frames = SRC_RATE;
    float *quiet = sine(441.0, 0.2, frames, SRC_RATE);
    float *mid = sine(441.0, 0.5, frames, SRC_RATE);
    float *loud = sine(441.0, 0.8, frames, SRC_RATE);
    char msg[160];
    float a, b, c;

    { /* 2. key zones */
        StSampleSet *set = st_sampleset_new();
        StRegion lo = region_template(0, 59, 57), hi = region_template(60, 127, 69);
        st_sampleset_add_region(set, &lo, quiet, frames, SRC_RATE);
        st_sampleset_add_region(set, &hi, loud, frames, SRC_RATE);
        a = level_of(set, 59.0f, 100, 0);
        b = level_of(set, 60.0f, 100, 0);
        c = level_of(set, 59.6f, 100, 0); /* rounds to 60 */
        snprintf(msg, sizeof msg, "note 59 -> %.2f, 60 -> %.2f, 59.6 -> %.2f",
                 a, b, c);
        check(fabsf(a - 0.2f) < 0.03f && fabsf(b - 0.8f) < 0.05f &&
              fabsf(c - 0.8f) < 0.05f, "zone selection", msg);
        st_sampleset_free(set);
    }
    { /* 3. velocity layers */
        StSampleSet *set = st_sampleset_new();
        StRegion soft = region_template(0, 127, 60), hard = soft;
        soft.hivel = 79;
        hard.lovel = 80;
        st_sampleset_add_region(set, &soft, quiet, frames, SRC_RATE);
        st_sampleset_add_region(set, &hard, loud, frames, SRC_RATE);
        a = level_of(set, 60.0f, 79, 0);
        b = level_of(set, 60.0f, 80, 0);
        snprintf(msg, sizeof msg, "vel 79 -> %.2f, vel 80 -> %.2f", a, b);
        check(fabsf(a - 0.2f) < 0.03f && fabsf(b - 0.8f) < 0.05f,
              "velocity layers", msg);
        /* and a key outside every region starts nothing */
        st_sampleset_free(set);
        set = st_sampleset_new();
        soft.lokey = 40;
        soft.hikey = 50;
        soft.hivel = 127;
        st_sampleset_add_region(set, &soft, quiet, frames, SRC_RATE);
        check(level_of(set, 60.0f, 100, 0) < 0.0f,
              "no matching region: start returns 0", NULL);
        st_sampleset_free(set);
    }
    { /* 4. round robin */
        StSampleSet *set = st_sampleset_new();
        StRegion r1 = region_template(0, 127, 60), r2 = r1;
        float lv[4];
        int i, ok = 1;
        r1.seq_length = r2.seq_length = 2;
        r1.seq_position = 1;
        r2.seq_position = 2;
        st_sampleset_add_region(set, &r1, quiet, frames, SRC_RATE);
        st_sampleset_add_region(set, &r2, mid, frames, SRC_RATE);
        for (i = 0; i < 4; i++) {
            lv[i] = level_of(set, 60.0f, 100, (unsigned)i);
            ok &= fabsf(lv[i] - (i % 2 ? 0.5f : 0.2f)) < 0.03f;
        }
        snprintf(msg, sizeof msg, "rr 0..3 -> %.2f %.2f %.2f %.2f", lv[0],
                 lv[1], lv[2], lv[3]);
        check(ok, "round robin alternates", msg);
        st_sampleset_free(set);
    }
    free(quiet);
    free(mid);
    free(loud);
}

/* ------------------------------------------------------------------ */
/* 5. Loop seam                                                        */

static void test_loop(void)
{
    /* 441 Hz at 44.1 kHz: exactly 100 frames a period. The loop holds 100
     * whole periods and the region starts at loop_start (offset=); every
     * frame outside the loop is zeroed, so a read across the seam into
     * the file (past loop_end, or before loop_start once wrapped) rather
     * than into the loop's continuation shows as a spike. Played at
     * 48 kHz the increment is fractional, so the 4-tap interpolation does
     * straddle the seam. */
    int frames = 30000, i, n = 3 * 48000, first_seam;
    float *x = sine(441.0, 0.5, frames, SRC_RATE);
    float *l = malloc((size_t)n * sizeof(float));
    float *r = malloc((size_t)n * sizeof(float));
    StSampleSet *set = st_sampleset_new();
    StRegion t = region_template(0, 127, 69);
    StSamplerPatch p = plain_patch();
    StSamplerVoice v;
    double inc = (double)SRC_RATE / 48000.0;
    float ref = 0.0f, all = 0.0f;
    int ended;
    char msg[200];

    t.loop = 1;
    t.loop_start = 10000;
    t.loop_end = 19999;
    t.offset = t.loop_start;
    for (i = 0; i < frames; i++)
        if (i < t.loop_start || i > t.loop_end)
            x[i] = 0.0f;
    st_sampleset_add_region(set, &t, x, frames, SRC_RATE);
    st_sampler_start(&v, set, &p, 48000.0f, 69.0f, 127, 0, 1);
    ended = render(&v, l, r, n);
    first_seam = (int)((t.loop_end + 1 - t.offset) / inc);
    /* second differences: the reference window is before any seam and
     * after the attack; the rest of the 3 s crosses 13 seams */
    for (i = 2400; i < n; i++) {
        float d2 = fabsf(l[i] - 2.0f * l[i - 1] + l[i - 2]);
        if (i < first_seam - 8) {
            if (d2 > ref)
                ref = d2;
        } else if (d2 > all) {
            all = d2;
        }
    }
    snprintf(msg, sizeof msg,
             "max |d2| across seams %.6f vs %.6f before (ratio %.3f), "
             "still sounding at 3 s: %s",
             all, ref, all / ref, ended == n ? "yes" : "no");
    check(all <= 1.5f * ref && ended == n, "loop seam is seamless", msg);
    check(peak(l + n - 4800, 4800) > 0.45f, "loop keeps its level", NULL);
    st_sampleset_free(set);
    free(x);
    free(l);
    free(r);
}

/* ------------------------------------------------------------------ */
/* 6. Release                                                          */

static void test_release(void)
{
    int frames = 2 * SRC_RATE, n = 48000, hold = 24000, i;
    float *x = sine(441.0, 0.5, frames, SRC_RATE);
    float *l = calloc((size_t)n, sizeof(float)), *r = calloc((size_t)n, sizeof(float));
    StSampleSet *set = st_sampleset_new();
    StRegion t = region_template(0, 127, 69);
    StSamplerPatch p = plain_patch();
    StSamplerVoice v;
    int at, end = -1;
    float step_before = 0.0f, step_after = 0.0f;
    char msg[200];

    t.loop = 1;
    t.loop_start = 1000;
    t.loop_end = 1000 + 100 * 50 - 1;
    st_sampleset_add_region(set, &t, x, frames, SRC_RATE);
    p.amp.release = 0.35f;
    st_sampler_start(&v, set, &p, 48000.0f, 69.0f, 127, 0, 1);
    for (at = 0; at < n; at += ST_BLOCK) {
        if (at == hold)
            st_sampler_release(&v);
        if (!st_sampler_render(&v, NULL, l + at, r + at, ST_BLOCK) && end < 0)
            end = at + ST_BLOCK;
    }
    for (i = hold - 4800; i < hold; i++)
        if (fabsf(l[i] - l[i - 1]) > step_before)
            step_before = fabsf(l[i] - l[i - 1]);
    for (i = hold; i < hold + 960; i++)
        if (fabsf(l[i] - l[i - 1]) > step_after)
            step_after = fabsf(l[i] - l[i - 1]);
    snprintf(msg, sizeof msg,
             "ended %.1f ms after note-off (release 350 ms, limit 385); "
             "largest step %.5f after vs %.5f before",
             end < 0 ? -1.0 : (end - hold) / 48.0, step_after, step_before);
    check(end > hold && end - hold <= (int)(0.385 * 48000) &&
              step_after <= step_before * 1.001f && !st_sampler_active(&v),
          "release decays and ends, no click", msg);
    check(peak(l + end, n - end) == 0.0f, "silent after the end", NULL);
    {
        /* the end itself is not a cut: the last 2 ms before the voice
         * went quiet are far below the held level */
        int last = end - 1;
        while (last > hold && l[last] == 0.0f)
            last--;
        snprintf(msg, sizeof msg, "last 2 ms peak %.6f vs held %.3f",
                 peak(l + last - 96, 96), peak(l + hold - 4800, 4800));
        check(peak(l + last - 96, 96) < 0.002f * peak(l + hold - 4800, 4800),
              "release fades to zero (no cut at the end)", msg);
    }
    st_sampleset_free(set);
    free(x);
    free(l);
    free(r);
}

/* ------------------------------------------------------------------ */
/* 7-8. Stack and determinism                                          */

static StSampleSet *stack_set(void)
{
    int frames = 2 * SRC_RATE;
    float *x = sine(220.5, 0.5, frames, SRC_RATE);
    StSampleSet *set = st_sampleset_new();
    StRegion t = region_template(0, 127, 57);
    t.loop = 1;
    t.loop_start = 10000;
    t.loop_end = 10000 + 200 * 100 - 1;
    st_sampleset_add_region(set, &t, x, frames, SRC_RATE);
    free(x);
    return set;
}

static void test_stack(void)
{
    StSampleSet *set = stack_set();
    StSamplerPatch p = plain_patch();
    StSamplerVoice v;
    static float l[48000], r[48000], l2[48000], r2[48000];
    double rl, rr_;
    char msg[160];

    p.stack = 3;
    p.stack_detune_cents = 12.0f;
    p.stack_delay = 0.03f;
    p.stereo_spread = 1.0f;
    st_sampler_start(&v, set, &p, 48000.0f, 57.0f, 100, 0, 1234);
    render(&v, l, r, 48000);
    rl = rms(l + 4800, 43200);
    rr_ = rms(r + 4800, 43200);
    snprintf(msg, sizeof msg, "RMS L %.4f R %.4f (ratio %.3f)", rl, rr_, rl / rr_);
    check(memcmp(l, r, sizeof l) != 0 && rl / rr_ > 0.75 && rl / rr_ < 1.33,
          "stack 3 spreads L/R with similar levels", msg);

    p.stack = 1;
    st_sampler_start(&v, set, &p, 48000.0f, 57.0f, 100, 0, 1234);
    render(&v, l, r, 48000);
    check(memcmp(l, r, sizeof l) == 0 && peak(l, 48000) > 0.1f,
          "stack 1 is centred (L == R exactly)", NULL);

    /* A note released before its delayed copies have spoken: they still
     * play (held as long as the note was), then the voice ends. */
    {
        int at, end = -1;
        double el, er;
        p.stack = 3;
        p.stack_delay = 0.05f;
        p.amp.release = 0.1f;
        st_sampler_start(&v, set, &p, 48000.0f, 57.0f, 100, 0, 5);
        memset(l, 0, sizeof l);
        memset(r, 0, sizeof r);
        for (at = 0; at < 48000; at += ST_BLOCK) {
            if (at == 480)
                st_sampler_release(&v);
            if (!st_sampler_render(&v, NULL, l + at, r + at, ST_BLOCK) &&
                end < 0)
                end = at + ST_BLOCK;
        }
        el = rms(l + 2400, 2400);
        er = rms(r + 2400, 2400);
        snprintf(msg, sizeof msg,
                 "released at 10 ms: RMS 50-100 ms L %.4f R %.4f, ended at %.0f ms",
                 el, er, end / 48.0);
        check(el > 0.001 && er > 0.001 && end > 0 && end <= (int)(0.17 * 48000),
              "early release still plays delayed copies, then ends", msg);
        p.stack_delay = 0.03f;
        p.amp.release = 0.35f;
    }

    /* 8. determinism: the same start renders the same bytes */
    p.stack = 3;
    st_sampler_start(&v, set, &p, 48000.0f, 57.3f, 90, 7, 99);
    render(&v, l, r, 48000);
    st_sampler_start(&v, set, &p, 48000.0f, 57.3f, 90, 7, 99);
    render(&v, l2, r2, 48000);
    check(memcmp(l, l2, sizeof l) == 0 && memcmp(r, r2, sizeof r) == 0,
          "determinism: identical renders", NULL);
    st_sampleset_free(set);
}

/* ------------------------------------------------------------------ */
/* 9-10. Real maps, failures, and the cost of loading every map        */

static int finite_all(const float *x, int n)
{
    int i;
    for (i = 0; i < n; i++)
        if (!isfinite(x[i]))
            return 0;
    return 1;
}

static void test_real_maps(void)
{
    char err[512], msg[600];
    StSampleSet *set = st_sfz_load("../samples/violins_sus.sfz", err, sizeof err);
    static float l[48000], r[48000];

    check(set != NULL, "load violins_sus.sfz", set ? NULL : err);
    if (set) {
        StSamplerPatch p = plain_patch();
        StSamplerVoice v;
        double f, dc;
        float pk;
        int fin;
        check(st_sampleset_region_count(set) == 22, "violins_sus has 22 regions",
              NULL);
        st_sampler_start(&v, set, &p, 48000.0f, 64.0f, 100, 0, 1);
        render(&v, l, r, 48000);
        fin = finite_all(l, 48000) && finite_all(r, 48000);
        pk = peak(l, 48000);
        f = ac_freq(l + 14400, 24000, 48000.0, 329.628);
        dc = cents(f, 329.628);
        snprintf(msg, sizeof msg,
                 "MIDI 64 vel 100: peak %.3f, finite %s, pitch %.2f Hz "
                 "(%+.1f cents from 329.63)",
                 pk, fin ? "yes" : "no", f, dc);
        check(fin && pk >= 0.05f && pk <= 1.5f, "violins play cleanly", msg);
        check(fabs(dc) <= 20.0, "violins pitch within 20 cents", msg);
        st_sampleset_free(set);
    }
    set = st_sfz_load("../samples/kit.sfz", err, sizeof err);
    check(set != NULL, "load kit.sfz", set ? NULL : err);
    if (set) {
        StSamplerPatch p = plain_patch();
        StSamplerVoice v;
        int ok = st_sampler_start(&v, set, &p, 48000.0f, 35.0f, 110, 0, 1);
        render(&v, l, r, 48000);
        snprintf(msg, sizeof msg, "%d regions, bass drum peak %.3f",
                 st_sampleset_region_count(set), peak(l, 48000));
        check(ok && finite_all(l, 48000) && peak(l, 48000) > 0.05f,
              "kit plays", msg);
        st_sampleset_free(set);
    }
    set = st_sfz_load("../samples/timpani.sfz", err, sizeof err);
    check(set != NULL, "load timpani.sfz", set ? NULL : err);
    if (set) {
        StSamplerPatch p = plain_patch();
        StSamplerVoice v;
        int ok = st_sampler_start(&v, set, &p, 48000.0f, 45.0f, 90, 0, 1);
        render(&v, l, r, 48000);
        snprintf(msg, sizeof msg, "%d regions, note 45 peak %.3f",
                 st_sampleset_region_count(set), peak(l, 48000));
        check(ok && finite_all(l, 48000) && peak(l, 48000) > 0.05f,
              "timpani plays", msg);
        st_sampleset_free(set);
    }

    /* 10. failures are clean and say why */
    err[0] = '\0';
    set = st_sfz_load("../samples/no_such_map.sfz", err, sizeof err);
    snprintf(msg, sizeof msg, "\"%s\"", err);
    check(set == NULL && strstr(err, "no_such_map.sfz") != NULL,
          "missing map fails with a message", msg);
    {
        const char *path = "/tmp/test_sampler_bad.sfz";
        FILE *f = fopen(path, "w");
        if (f) {
            fputs("<region> sample=nowhere/missing.flac key=60\n", f);
            fclose(f);
        }
        err[0] = '\0';
        set = st_sfz_load(path, err, sizeof err);
        snprintf(msg, sizeof msg, "\"%s\"", err);
        check(set == NULL && strstr(err, "missing.flac") != NULL,
              "missing sample fails with a message", msg);
        f = fopen(path, "w");
        if (f) {
            fputs("<region> sample=x.wav key=60\n", f);
            fclose(f);
        }
        err[0] = '\0';
        set = st_sfz_load(path, err, sizeof err);
        snprintf(msg, sizeof msg, "\"%s\"", err);
        check(set == NULL && strstr(err, "FLAC") != NULL,
              "a non-FLAC sample is refused by name", msg);
        remove(path);
    }
}

/* Header inheritance, note names, several opcodes per line, comments,
 * <control> default_path and an unknown header, on a map written to /tmp
 * whose default_path points back at the real samples. */
static void test_parser(void)
{
    const char *path = "/tmp/test_sampler_parse.sfz";
    char err[512], msg[600];
    char cwd[400];
    FILE *f = fopen(path, "w");
    StSampleSet *set;
    if (!f || !getcwd(cwd, sizeof cwd)) {
        check(0, "parser: write a temporary map", path);
        if (f)
            fclose(f);
        return;
    }
    fprintf(f, "// a comment line\n"
               "<control> default_path=%s/../samples/\n", cwd);
    fputs("<global> volume=-6 tune=5 // trailing comment\n"
          "<group> lovel=1 hivel=63 seq_length=2\n"
          "<region> sample=percussion/kit/kit_56.flac key=c4 seq_position=1\n"
          "<region> sample=percussion/kit/kit_56.flac lokey=c#4 hikey=d4"
          " pitch_keycenter=61 seq_position=2 tune=-7 offset=10\n"
          "<group> lovel=64\n"
          "<region>\n"
          "sample=percussion/kit/kit_55.flac lokey=50 hikey=70\n"
          "<curve> v000=0 v127=1\n",
          f);
    fclose(f);
    set = st_sfz_load(path, err, sizeof err);
    check(set != NULL, "parser: load a hand-written map", set ? NULL : err);
    if (set) {
        const StRegion *a = st_sampleset_region(set, 0);
        const StRegion *b = st_sampleset_region(set, 1);
        const StRegion *c = st_sampleset_region(set, 2);
        int ok = st_sampleset_region_count(set) == 3 &&
                 a->lokey == 60 && a->hikey == 60 && a->keycenter == 60 &&
                 a->volume_db == -6.0f && a->tune_cents == 5.0f &&
                 a->hivel == 63 && a->seq_length == 2 && a->seq_position == 1 &&
                 b->lokey == 61 && b->hikey == 62 && b->tune_cents == -7.0f &&
                 b->offset == 10 && b->seq_position == 2 &&
                 a->data == b->data && /* the same file, decoded once */
                 c->lovel == 64 && c->hivel == 127 && c->seq_length == 1 &&
                 c->volume_db == -6.0f && c->lokey == 50 && c->data != a->data;
        snprintf(msg, sizeof msg, "%d regions, inheritance/note names/dedup %s",
                 st_sampleset_region_count(set), ok ? "right" : "WRONG");
        check(ok, "parser: headers, opcodes, names, dedup", msg);
        st_sampleset_free(set);
    }
    remove(path);
}

static void measure_all_maps(void)
{
    DIR *d = opendir("../samples");
    struct dirent *e;
    double t0 = now(), secs;
    long long frames = 0;
    int maps = 0, regions = 0, ok = 1;
    char msg[300];
    if (!d) {
        check(0, "open ../samples", NULL);
        return;
    }
    while ((e = readdir(d)) != NULL) {
        char path[600], err[512];
        size_t n = strlen(e->d_name);
        StSampleSet *set;
        int i, j;
        if (n < 5 || strcmp(e->d_name + n - 4, ".sfz") != 0)
            continue;
        snprintf(path, sizeof path, "../samples/%s", e->d_name);
        set = st_sfz_load(path, err, sizeof err);
        if (!set) {
            check(0, "load map", err);
            ok = 0;
            continue;
        }
        maps++;
        for (i = 0; i < st_sampleset_region_count(set); i++) {
            const StRegion *ri = st_sampleset_region(set, i);
            int seen = 0;
            for (j = 0; j < i; j++)
                if (st_sampleset_region(set, j)->data == ri->data)
                    seen = 1;
            if (!seen)
                frames += ri->frames;
            regions++;
        }
        st_sampleset_free(set);
    }
    closedir(d);
    secs = now() - t0;
    snprintf(msg, sizeof msg,
             "%d maps, %d regions, %lld frames decoded in %.2f s; "
             "%.1f MB as float",
             maps, regions, frames, secs, frames * 4.0 / (1024.0 * 1024.0));
    check(ok && maps > 0, "load every map", msg);
}

int main(void)
{
    test_pitch();
    test_selection();
    test_loop();
    test_release();
    test_stack();
    test_real_maps();
    test_parser();
    measure_all_maps();
    printf("%s (%d failure%s)\n", failures ? "FAILED" : "all passed", failures,
           failures == 1 ? "" : "s");
    return failures ? 1 : 0;
}
