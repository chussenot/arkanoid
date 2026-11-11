/*
 * test_synth.c: checks for the studio synth voice. BRYX addition (see
 * studio.h).
 *
 *   cd audio/engine && cc -std=c99 -O2 -Wall -Wextra -Werror \
 *       -o /tmp/test_synth tests/test_synth.c studio_synth.c \
 *       studio_common.c -lm && /tmp/test_synth
 *
 * Prints one line per check; exits non-zero if any fails.
 */
#include <math.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "../studio_synth.h"

static int failures;
static int checks;

static void check(int ok, const char *fmt, ...)
{
    va_list ap;
    checks++;
    printf("%s ", ok ? "  ok  " : "  FAIL");
    va_start(ap, fmt);
    vprintf(fmt, ap);
    va_end(ap);
    printf("\n");
    if (!ok)
        failures++;
}

#define PI 3.14159265358979323846

/* ------------------------------------------------------------------ */
/* Helpers.                                                            */

static float *buf(int n)
{
    float *b = calloc((size_t)n, sizeof *b);
    if (!b) {
        fprintf(stderr, "out of memory\n");
        exit(2);
    }
    return b;
}

/* A patch with everything that would colour a measurement switched off:
 * instant attack, full sustain, no filter, LFOs unrouted. */
static void plain_patch(StSynthPatch *p, StWave wave)
{
    st_synth_patch_default(p);
    p->wave = wave;
    p->amp = (StAdsr){0.0f, 0.0f, 1.0f, 0.1f};
    p->filter = ST_FILTER_NONE;
    p->key_track = 0.0f;
}

static float hz_to_note(double hz)
{
    return (float)(69.0 + 12.0 * log2(hz / 440.0));
}

/* Render n frames in calls of `chunk` frames with unity modulation.
 * Returns the last render's result. */
static int render(StSynthVoice *v, float *l, float *r, int n, int chunk)
{
    StVoiceMod mod = {1.0f, 1.0f, 0.0f, 0.0f};
    int alive = 1;
    for (int i = 0; i < n; i += chunk) {
        int m = n - i < chunk ? n - i : chunk;
        alive = st_synth_render(v, &mod, l + i, r + i, m);
    }
    return alive;
}

/* Upward zero crossings (linearly interpolated, in samples) in
 * [from, to). Returns the count; times go to t (up to max). */
static int crossings(const float *x, int from, int to, double *t, int max)
{
    int c = 0;
    for (int i = from + 1; i < to && c < max; i++) {
        if (x[i - 1] < 0.0f && x[i] >= 0.0f) {
            double a = x[i - 1], b = x[i];
            t[c++] = (i - 1) + (-a) / (b - a);
        }
    }
    return c;
}

/* Mean frequency over [from, to) from the first and last crossing. */
static double mean_freq(const float *x, int from, int to, double rate)
{
    static double t[200000];
    int c = crossings(x, from, to, t, 200000);
    if (c < 2)
        return 0.0;
    return (c - 1) * rate / (t[c - 1] - t[0]);
}

/* Frequency of the cycle spanning sample `at`. */
static double freq_at(const float *x, int n, int at, double rate)
{
    static double t[200000];
    int c = crossings(x, 0, n, t, 200000);
    for (int i = 1; i < c; i++)
        if (t[i] >= at)
            return rate / (t[i] - t[i - 1]);
    return 0.0;
}

static double cents(double f, double ref)
{
    return 1200.0 * log2(f / ref);
}

static double rms(const float *x, int from, int to)
{
    double s = 0.0;
    for (int i = from; i < to; i++)
        s += (double)x[i] * x[i];
    return sqrt(s / (to - from));
}

static double db(double ratio)
{
    return 20.0 * log10(ratio);
}

/* Power of the component at f (rectangular window, f on a bin). */
static double tone_power(const float *x, int n, double f, double rate)
{
    double re = 0.0, im = 0.0, w = 2.0 * PI * f / rate;
    for (int i = 0; i < n; i++) {
        re += x[i] * cos(w * i);
        im -= x[i] * sin(w * i);
    }
    return 2.0 * (re * re + im * im) / ((double)n * n);
}

/* Spectral centroid (Hz) of a Hann-windowed frame of 1024 samples. */
static double centroid(const float *x, double rate)
{
    enum { N = 1024 };
    double num = 0.0, den = 0.0;
    for (int k = 1; k < N / 2; k++) {
        double re = 0.0, im = 0.0;
        for (int i = 0; i < N; i++) {
            double w = 0.5 - 0.5 * cos(2.0 * PI * i / (N - 1));
            double a = 2.0 * PI * k * i / N;
            re += w * x[i] * cos(a);
            im -= w * x[i] * sin(a);
        }
        double mag = sqrt(re * re + im * im);
        num += mag * k * rate / N;
        den += mag;
    }
    return den > 0.0 ? num / den : 0.0;
}

static double peak(const float *x, int from, int to)
{
    double p = 0.0;
    for (int i = from; i < to; i++)
        if (fabs(x[i]) > p)
            p = fabs(x[i]);
    return p;
}

/* ------------------------------------------------------------------ */
/* 1. Pitch accuracy.                                                  */

static void test_pitch(void)
{
    printf("1. pitch accuracy\n");
    const float rates[2] = {48000.0f, 44100.0f};
    const StWave waves[2] = {ST_WAVE_SAW, ST_WAVE_SINE};
    const double freqs[2] = {440.0, 55.0};
    for (int ri = 0; ri < 2; ri++)
        for (int wi = 0; wi < 2; wi++)
            for (int fi = 0; fi < 2; fi++) {
                float rate = rates[ri];
                int n = (int)(rate * 1.2f);
                float *l = buf(n), *r = buf(n);
                StSynthPatch p;
                plain_patch(&p, waves[wi]);
                StSynthVoice v;
                st_synth_start(&v, &p, rate, hz_to_note(freqs[fi]), 1.0f, 7);
                render(&v, l, r, n, ST_BLOCK);
                int from = (int)(0.1f * rate);
                double f = mean_freq(l, from, from + (int)rate, rate);
                double c = cents(f, freqs[fi]);
                check(fabs(c) < 1.0, "%s %5.0f Hz at %5.0f Hz: %+.4f cents",
                      wi == 0 ? "saw " : "sine", freqs[fi], rate, c);
                free(l);
                free(r);
            }
}

/* ------------------------------------------------------------------ */
/* 2. Anti-aliasing.                                                   */

/* Alias-to-harmonic ratio (dB) of x: harmonics of f0 below Nyquist are
 * measured by DFT; everything else (minus DC) is aliasing. */
static double alias_db(const float *x, int n, double f0, double rate,
                       double *alias_power)
{
    double mean = 0.0, total = 0.0, harm = 0.0;
    for (int i = 0; i < n; i++)
        mean += x[i];
    mean /= n;
    for (int i = 0; i < n; i++)
        total += ((double)x[i] - mean) * ((double)x[i] - mean);
    total /= n;
    for (int k = 1; k * f0 < rate / 2.0; k++)
        harm += tone_power(x, n, k * f0, rate);
    *alias_power = total - harm;
    return 10.0 * log10(harm / (total - harm));
}

static void test_alias(void)
{
    printf("2. anti-aliasing (saw at 3520 Hz, 48 kHz)\n");
    const double rate = 48000.0, f0 = 3520.0;
    int n = 48000, skip = 4800;
    float *l = buf(n + skip), *r = buf(n + skip), *naive = buf(n);
    StSynthPatch p;
    plain_patch(&p, ST_WAVE_SAW);
    StSynthVoice v;
    st_synth_start(&v, &p, (float)rate, hz_to_note(f0), 1.0f, 3);
    render(&v, l, r, n + skip, ST_BLOCK);
    for (int i = 0; i < n; i++) {
        double t = fmod((double)i * f0 / rate, 1.0);
        naive[i] = (float)(2.0 * t - 1.0);
    }
    double ap_blep, ap_naive;
    double rb = alias_db(l + skip, n, f0, rate, &ap_blep);
    double rn = alias_db(naive, n, f0, rate, &ap_naive);
    double cut = 10.0 * log10(ap_naive / ap_blep);
    printf("        harmonics/alias: PolyBLEP %.1f dB, naive %.1f dB\n", rb,
           rn);
    check(rb >= 25.0 || cut >= 15.0,
          "aliasing %.1f dB below harmonics; PolyBLEP cuts it by %.1f dB "
          "vs naive (need >= 25 or >= 15)",
          rb, cut);

    /* The triangle (PolyBLAMP) and square (PolyBLEP) at the same pitch. */
    const StWave w2[2] = {ST_WAVE_TRIANGLE, ST_WAVE_SQUARE};
    const char *nm[2] = {"triangle", "square"};
    for (int wi = 0; wi < 2; wi++) {
        plain_patch(&p, w2[wi]);
        memset(l, 0, sizeof(float) * (size_t)(n + skip));
        memset(r, 0, sizeof(float) * (size_t)(n + skip));
        st_synth_start(&v, &p, (float)rate, hz_to_note(f0), 1.0f, 3);
        render(&v, l, r, n + skip, ST_BLOCK);
        for (int i = 0; i < n; i++) {
            double t = fmod((double)i * f0 / rate, 1.0);
            naive[i] = wi == 0 ? (float)(1.0 - 4.0 * fabs(t - 0.5))
                               : (t < 0.5 ? 1.0f : -1.0f);
        }
        double a1, a2;
        double rb2 = alias_db(l + skip, n, f0, rate, &a1);
        double rn2 = alias_db(naive, n, f0, rate, &a2);
        check(rb2 > rn2 + 6.0, "%s: aliasing %.1f dB below harmonics "
              "(naive %.1f dB)", nm[wi], rb2, rn2);
    }
    free(l);
    free(r);
    free(naive);
}

/* ------------------------------------------------------------------ */
/* 3. Filters.                                                         */

/* Gain (dB) of `mode` at cutoff `fc` on a full-scale sine at f, at
 * `rate`. */
static double filter_gain_db(StFilterMode mode, float fc, float res,
                             double f, float rate)
{
    int n = (int)(rate * 0.6f), from = (int)(rate * 0.2f);
    float *l = buf(n), *r = buf(n), *l0 = buf(n), *r0 = buf(n);
    StSynthPatch p;
    plain_patch(&p, ST_WAVE_SINE);
    StSynthVoice v;
    st_synth_start(&v, &p, rate, hz_to_note(f), 1.0f, 1);
    render(&v, l0, r0, n, ST_BLOCK);
    p.filter = mode;
    p.cutoff_hz = fc;
    p.resonance = res;
    st_synth_start(&v, &p, rate, hz_to_note(f), 1.0f, 1);
    render(&v, l, r, n, ST_BLOCK);
    double g = db(rms(l, from, n) / rms(l0, from, n));
    free(l);
    free(r);
    free(l0);
    free(r0);
    return g;
}

static void test_filters(void)
{
    printf("3. filters\n");
    double lp1 = filter_gain_db(ST_FILTER_LP, 1000.0f, 0.0f, 1000.0, 48000);
    double lp4 = filter_gain_db(ST_FILTER_LP, 1000.0f, 0.0f, 4000.0, 48000);
    check(fabs(lp1 + 3.0) <= 1.0, "SVF LP 1 kHz, res 0: %.2f dB at 1 kHz", lp1);
    check(lp4 <= -20.0, "SVF LP 1 kHz, res 0: %.2f dB at 4 kHz", lp4);
    double hp1 = filter_gain_db(ST_FILTER_HP, 1000.0f, 0.0f, 1000.0, 48000);
    double hp4 = filter_gain_db(ST_FILTER_HP, 1000.0f, 0.0f, 250.0, 48000);
    check(fabs(hp1 + 3.0) <= 1.0, "SVF HP 1 kHz, res 0: %.2f dB at 1 kHz", hp1);
    check(hp4 <= -20.0, "SVF HP 1 kHz, res 0: %.2f dB at 250 Hz", hp4);
    double bp1 = filter_gain_db(ST_FILTER_BP, 1000.0f, 0.5f, 1000.0, 48000);
    double bp4 = filter_gain_db(ST_FILTER_BP, 1000.0f, 0.5f, 4000.0, 48000);
    check(fabs(bp1) <= 1.0 && bp4 < -12.0,
          "SVF BP 1 kHz, res 0.5: %.2f dB at 1 kHz, %.2f dB at 4 kHz", bp1,
          bp4);

    /* SVF resonance. In its linear range the peak at cutoff is Q: the
     * 20th harmonic of a 50 Hz saw (amplitude 0.03) sitting on a 1 kHz
     * cutoff rises by Q / 0.707 against resonance 0. A full-scale sine at
     * the cutoff drives the band integrator into its knee: loud, bounded. */
    {
        const float rate = 48000.0f;
        int n = (int)rate, skip = (int)(0.5f * rate);
        double h[2];
        const float res[2] = {0.0f, 0.8f};
        for (int i = 0; i < 2; i++) {
            float *l = buf(n + skip), *r = buf(n + skip);
            StSynthPatch p;
            plain_patch(&p, ST_WAVE_SAW);
            p.filter = ST_FILTER_LP;
            p.cutoff_hz = 1000.0f;
            p.resonance = res[i];
            StSynthVoice v;
            st_synth_start(&v, &p, rate, hz_to_note(50.0), 1.0f, 1);
            render(&v, l, r, n + skip, ST_BLOCK);
            h[i] = tone_power(l + skip, n, 1000.0, rate);
            free(l);
            free(r);
        }
        double want = db(1.0 / (sqrt(2.0) * pow(0.025, 0.8)) / sqrt(0.5));
        double got = 10.0 * log10(h[1] / h[0]);
        check(fabs(got - want) < 1.5,
              "SVF LP res 0.8 (Q %.1f): the harmonic on the cutoff rises "
              "%+.1f dB (linear theory %+.1f dB)",
              1.0 / (sqrt(2.0) * pow(0.025, 0.8)), got, want);
        double loud = filter_gain_db(ST_FILTER_LP, 1000.0f, 0.8f, 1000.0,
                                     48000);
        check(loud > 3.0 && loud < 10.0,
              "SVF LP res 0.8, full-scale sine on the cutoff: %+.1f dB "
              "(saturating, bounded)", loud);
    }

    const double probe[3] = {1000.0, 4000.0, 8000.0};
    for (int i = 0; i < 3; i++) {
        double a = filter_gain_db(ST_FILTER_LP, 1000.0f, 0.0f, probe[i], 48000);
        double b = filter_gain_db(ST_FILTER_LP, 1000.0f, 0.0f, probe[i], 44100);
        check(fabs(a - b) <= 1.0,
              "SVF LP same at 48/44.1 kHz at %.0f Hz: %.2f / %.2f dB",
              probe[i], a, b);
    }
    double la = filter_gain_db(ST_FILTER_LADDER, 1000.0f, 0.5f, 1000.0, 48000);
    double lb = filter_gain_db(ST_FILTER_LADDER, 1000.0f, 0.5f, 1000.0, 44100);
    check(fabs(la - lb) <= 1.0, "ladder same at 48/44.1 kHz: %.2f / %.2f dB",
          la, lb);

    double l0 = filter_gain_db(ST_FILTER_LADDER, 1000.0f, 0.0f, 1000.0, 48000);
    double l9 = filter_gain_db(ST_FILTER_LADDER, 1000.0f, 0.95f, 1000.0, 48000);
    double l4 = filter_gain_db(ST_FILTER_LADDER, 1000.0f, 0.0f, 4000.0, 48000);
    check(l9 - l0 > 6.0,
          "ladder resonant peak at cutoff: res 0 %.2f dB, res 0.95 %.2f dB "
          "(+%.1f dB)",
          l0, l9, l9 - l0);
    check(l4 - l0 < -30.0, "ladder 4-pole slope: %.1f dB from 1 to 4 kHz",
          l4 - l0);

    /* Bounded: a ladder at resonance 1 fed a loud saw, 3 s. */
    StSynthPatch p;
    plain_patch(&p, ST_WAVE_SAW);
    p.filter = ST_FILTER_LADDER;
    p.resonance = 1.0f;
    p.cutoff_hz = 800.0f;
    int n = 48000 * 3;
    float *l = buf(n), *r = buf(n);
    StSynthVoice v;
    st_synth_start(&v, &p, 48000.0f, 36.0f, 1.0f, 9);
    render(&v, l, r, n, ST_BLOCK);
    double pk = peak(l, 0, n);
    check(isfinite(pk) && pk < 4.0, "ladder at resonance 1 stays bounded: "
          "peak %.3f", pk);
    free(l);
    free(r);
}

/* ------------------------------------------------------------------ */
/* 4. Envelopes.                                                       */

static void test_envelopes(void)
{
    printf("4. envelopes\n");
    const float rate = 48000.0f;
    StSynthPatch p;
    plain_patch(&p, ST_WAVE_SINE);
    p.amp = (StAdsr){0.1f, 0.0f, 1.0f, 0.3f};
    int n = (int)(rate * 2.0f);
    float *l = buf(n), *r = buf(n);
    StSynthVoice v;
    st_synth_start(&v, &p, rate, 69.0f, 1.0f, 5);
    int a = (int)(0.11f * rate);
    render(&v, l, r, a, ST_BLOCK);
    check(v.amp_env.level > 0.95f, "attack 0.1 s: envelope %.4f at 0.11 s",
          v.amp_env.level);
    double pk_att = peak(l, a - 240, a);
    check(pk_att > 0.95, "attack 0.1 s: output peak %.4f in 0.105..0.11 s",
          pk_att);
    int rel = (int)(0.5f * rate);
    render(&v, l + a, r + a, rel - a, ST_BLOCK);
    double sus = peak(l, rel - 4800, rel);
    st_synth_release(&v);
    int t35 = rel + (int)(0.35f * rate);
    render(&v, l + rel, r + rel, t35 - rel, ST_BLOCK);
    double tail = peak(l, t35 - 480, t35);
    check(db(tail / sus) < -40.0, "release 0.3 s: %.1f dB at 0.35 s",
          db(tail / sus));
    int alive = 1, end = t35;
    while (alive && end < n) {
        alive = render(&v, l + end, r + end, ST_BLOCK, ST_BLOCK);
        end += ST_BLOCK;
    }
    check(!alive && !st_synth_active(&v),
          "render returns 0 once the release ends (%.3f s after note off)",
          (double)(end - rel) / rate);
    float z[64] = {0}, zr[64] = {0};
    int again = st_synth_render(&v, NULL, z, zr, 64);
    int silent = 1;
    for (int i = 0; i < 64; i++)
        silent &= z[i] == 0.0f && zr[i] == 0.0f;
    check(again == 0 && silent, "a finished voice returns 0 and adds nothing");
    free(l);
    free(r);
}

/* ------------------------------------------------------------------ */
/* 5. The filter envelope opens the sound.                             */

static void test_filter_env(void)
{
    printf("5. filter envelope\n");
    const float rate = 48000.0f;
    StSynthPatch p;
    plain_patch(&p, ST_WAVE_SAW);
    p.amp = (StAdsr){0.002f, 0.0f, 1.0f, 0.2f};
    p.filter = ST_FILTER_LP;
    p.cutoff_hz = 250.0f;
    p.resonance = 0.2f;
    p.env_amount_oct = 4.0f;
    p.filter_env = (StAdsr){0.001f, 0.2f, 0.0f, 0.2f};
    int n = (int)(rate * 0.6f);
    float *l = buf(n), *r = buf(n);
    StSynthVoice v;
    st_synth_start(&v, &p, rate, 48.0f, 1.0f, 11);
    render(&v, l, r, n, ST_BLOCK);
    double c20 = centroid(l + (int)(0.020f * rate), rate);
    double c500 = centroid(l + (int)(0.500f * rate), rate);
    check(c20 > c500 * 1.5,
          "pluck: centroid %.0f Hz at 20 ms > %.0f Hz at 500 ms", c20, c500);
    free(l);
    free(r);
}

/* ------------------------------------------------------------------ */
/* 6. Stack and stereo.                                                */

static void test_stack(void)
{
    printf("6. stack and stereo\n");
    const float rate = 48000.0f;
    int n = (int)rate;
    float *l = buf(n), *r = buf(n);
    StSynthPatch p;
    plain_patch(&p, ST_WAVE_SAW);
    p.stack = 5;
    p.detune_cents = 20.0f;
    p.stereo_spread = 1.0f;
    StSynthVoice v;
    st_synth_start(&v, &p, rate, 57.0f, 1.0f, 21);
    render(&v, l, r, n, ST_BLOCK);
    double slr = 0.0, sll = 0.0, srr = 0.0;
    for (int i = 0; i < n; i++) {
        slr += (double)l[i] * r[i];
        sll += (double)l[i] * l[i];
        srr += (double)r[i] * r[i];
    }
    double corr = slr / sqrt(sll * srr);
    double bal = db(sqrt(sll / srr));
    check(corr < 0.95, "stack 5, detune 20, spread 1: L/R correlation %.3f",
          corr);
    check(fabs(bal) < 1.0, "stack 5: L/R RMS balance %+.2f dB", bal);
    double one = rms(l, 0, n);
    printf("        stack 5 RMS %.3f (a single saw is 0.577)\n", one);

    /* stack 1 (through drive and the SVF) and a spread-0 stack: L == R. */
    const int stacks[2] = {1, 7};
    for (int si = 0; si < 2; si++) {
        memset(l, 0, sizeof(float) * (size_t)n);
        memset(r, 0, sizeof(float) * (size_t)n);
        plain_patch(&p, ST_WAVE_SAW);
        p.stack = stacks[si];
        p.detune_cents = 30.0f;
        p.stereo_spread = si == 0 ? 1.0f : 0.0f;
        p.filter = ST_FILTER_LP;
        p.cutoff_hz = 2000.0f;
        p.drive = 0.5f;
        p.noise_level = 0.1f;
        p.sub_level = 0.5f;
        st_synth_start(&v, &p, rate, 45.0f, 0.8f, 4);
        render(&v, l, r, n, ST_BLOCK);
        check(memcmp(l, r, sizeof(float) * (size_t)n) == 0,
              "stack %d, spread %.0f: L == R exactly", p.stack,
              p.stereo_spread);
    }
    free(l);
    free(r);
}

/* ------------------------------------------------------------------ */
/* 7. Glide.                                                           */

static void test_glide(void)
{
    printf("7. glide\n");
    const float rate = 48000.0f;
    StSynthPatch p;
    plain_patch(&p, ST_WAVE_SINE);
    p.glide_time = 0.1f;
    p.mono = 1;
    int pre = (int)(0.2f * rate), n = pre + (int)(0.3f * rate);
    float *l = buf(n), *r = buf(n);
    StSynthVoice v;
    st_synth_start(&v, &p, rate, 48.0f, 1.0f, 2);
    render(&v, l, r, pre, ST_BLOCK);
    st_synth_legato(&v, 60.0f, 1.0f);
    render(&v, l + pre, r + pre, n - pre, ST_BLOCK);
    double n50 = hz_to_note(freq_at(l, n, pre + (int)(0.05f * rate), rate));
    double n150 = hz_to_note(freq_at(l, n, pre + (int)(0.15f * rate), rate));
    check(n50 > 48.5 && n50 < 59.5, "legato 48 -> 60, glide 0.1 s: note "
          "%.2f at 50 ms", n50);
    check(fabs(n150 - 60.0) * 100.0 < 5.0, "... note %.3f at 150 ms "
          "(%+.2f cents)", n150, (n150 - 60.0) * 100.0);
    check(v.amp_env.stage == 3, "legato did not retrigger the envelope");

    /* glide 0 jumps. */
    p.glide_time = 0.0f;
    memset(l, 0, sizeof(float) * (size_t)n);
    st_synth_start(&v, &p, rate, 48.0f, 1.0f, 2);
    render(&v, l, r, pre, ST_BLOCK);
    st_synth_legato(&v, 60.0f, 1.0f);
    render(&v, l + pre, r + pre, n - pre, ST_BLOCK);
    double j = hz_to_note(freq_at(l, n, pre + (int)(0.01f * rate), rate));
    check(fabs(j - 60.0) < 0.05, "glide 0: note %.3f 10 ms after legato", j);
    free(l);
    free(r);
}

/* ------------------------------------------------------------------ */
/* 8. LFO vibrato.                                                     */

static void test_vibrato(void)
{
    printf("8. LFO vibrato\n");
    const float rate = 48000.0f;
    StSynthPatch p;
    plain_patch(&p, ST_WAVE_SINE);
    p.lfo[0].shape = ST_LFO_SINE;
    p.lfo[0].rate_hz = 5.0f;
    p.lfo[0].to_pitch_cents = 30.0f;
    int n = (int)(2.2f * rate);
    float *l = buf(n), *r = buf(n);
    StSynthVoice v;
    st_synth_start(&v, &p, rate, 69.0f, 1.0f, 8);
    render(&v, l, r, n, ST_BLOCK);
    static double t[4000];
    int c = crossings(l, (int)(0.2f * rate), (int)(2.2f * rate), t, 4000);
    double hi = -1e9, lo = 1e9;
    int flips = 0, sign = 0;
    for (int i = 1; i < c; i++) {
        double dev = cents(rate / (t[i] - t[i - 1]), 440.0);
        if (dev > hi)
            hi = dev;
        if (dev < lo)
            lo = dev;
        int s = dev > 10.0 ? 1 : (dev < -10.0 ? -1 : 0);
        if (s && s != sign) {
            if (sign)
                flips++;
            sign = s;
        }
    }
    check(hi > 24.0 && hi < 36.0 && lo < -24.0 && lo > -36.0,
          "5 Hz, 30 cents: deviation %+.1f .. %+.1f cents", lo, hi);
    /* 2 s at 5 Hz: about 20 sign changes. */
    check(flips >= 18 && flips <= 21, "periodic: %d sign changes in 2 s "
          "(5 Hz -> ~20)", flips);

    /* Delay: no vibrato before the delay, full depth after twice it. */
    p.lfo[0].delay = 0.3f;
    memset(l, 0, sizeof(float) * (size_t)n);
    st_synth_start(&v, &p, rate, 69.0f, 1.0f, 8);
    render(&v, l, r, n, ST_BLOCK);
    c = crossings(l, 0, n, t, 4000);
    double early = 0.0, late = 0.0;
    for (int i = 1; i < c; i++) {
        double dev = fabs(cents(rate / (t[i] - t[i - 1]), 440.0));
        if (t[i] < 0.28 * rate && dev > early)
            early = dev;
        if (t[i] > 0.7 * rate && dev > late)
            late = dev;
    }
    check(early < 1.0 && late > 24.0, "delay 0.3 s: %.2f cents before, "
          "%.1f cents after 0.7 s", early, late);
    free(l);
    free(r);
}

/* ------------------------------------------------------------------ */
/* 9. Determinism.                                                     */

static void rich_patch(StSynthPatch *p)
{
    st_synth_patch_default(p);
    p->wave = ST_WAVE_PULSE;
    p->pulse_width = 0.3f;
    p->stack = 3;
    p->detune_cents = 15.0f;
    p->stereo_spread = 0.7f;
    p->noise_level = 0.1f;
    p->sub_level = 0.3f;
    p->filter = ST_FILTER_LADDER;
    p->cutoff_hz = 1200.0f;
    p->resonance = 0.6f;
    p->env_amount_oct = 2.0f;
    p->drive = 0.3f;
    p->lfo[0].shape = ST_LFO_SAMPLE_HOLD;
    p->lfo[0].rate_hz = 7.0f;
    p->lfo[0].to_cutoff_oct = 1.0f;
    p->lfo[1].shape = ST_LFO_TRIANGLE;
    p->lfo[1].rate_hz = 0.7f;
    p->lfo[1].to_pulse_width = 0.2f;
    p->lfo[1].to_pan = 0.5f;
}

static void test_determinism(void)
{
    printf("9. determinism\n");
    const float rate = 48000.0f;
    int n = (int)rate;
    size_t bytes = sizeof(float) * (size_t)n;
    float *l1 = buf(n), *r1 = buf(n), *l2 = buf(n), *r2 = buf(n);
    StSynthPatch p;
    rich_patch(&p);
    StSynthVoice v;
    st_synth_start(&v, &p, rate, 50.0f, 0.9f, 42);
    render(&v, l1, r1, n, ST_BLOCK);
    st_synth_start(&v, &p, rate, 50.0f, 0.9f, 42);
    render(&v, l2, r2, n, ST_BLOCK);
    check(!memcmp(l1, l2, bytes) && !memcmp(r1, r2, bytes),
          "same seed: identical bytes");
    memset(l2, 0, bytes);
    memset(r2, 0, bytes);
    st_synth_start(&v, &p, rate, 50.0f, 0.9f, 43);
    render(&v, l2, r2, n, ST_BLOCK);
    check(memcmp(l1, l2, bytes) != 0, "different seed, stack 3: differs");

    /* How a render is split into calls does not matter. */
    const int chunks[2] = {1, 1000};
    for (int ci = 0; ci < 2; ci++) {
        memset(l2, 0, bytes);
        memset(r2, 0, bytes);
        st_synth_start(&v, &p, rate, 50.0f, 0.9f, 42);
        render(&v, l2, r2, n, chunks[ci]);
        check(!memcmp(l1, l2, bytes) && !memcmp(r1, r2, bytes),
              "calls of %d frames render the same bytes as calls of %d",
              chunks[ci], ST_BLOCK);
    }
    free(l1);
    free(r1);
    free(l2);
    free(r2);
}

/* ------------------------------------------------------------------ */
/* 10. Stress.                                                         */

static void test_stress(void)
{
    printf("10. stress\n");
    const StFilterMode modes[5] = {ST_FILTER_NONE, ST_FILTER_LP, ST_FILTER_HP,
                                   ST_FILTER_BP, ST_FILTER_LADDER};
    const char *names[5] = {"none", "LP", "HP", "BP", "ladder"};
    const StWave waves[6] = {ST_WAVE_SAW, ST_WAVE_SQUARE, ST_WAVE_PULSE,
                             ST_WAVE_TRIANGLE, ST_WAVE_SINE, ST_WAVE_NOISE};
    const float rates[2] = {48000.0f, 44100.0f};
    for (int ri = 0; ri < 2; ri++)
        for (int mi = 0; mi < 5; mi++) {
            float rate = rates[ri];
            int n = (int)(5.0f * rate);
            float *l = buf(n), *r = buf(n);
            StSynthPatch p;
            st_synth_patch_default(&p);
            p.wave = waves[(mi + ri) % 6];
            p.pulse_width = 0.05f;
            p.stack = 7;
            p.detune_cents = 100.0f;
            p.stereo_spread = 1.0f;
            p.sub_level = 1.0f;
            p.noise_level = 1.0f;
            p.amp = (StAdsr){0.0f, 0.05f, 1.0f, 0.05f};
            p.filter = modes[mi];
            p.cutoff_hz = 20000.0f;
            p.resonance = 1.0f;
            p.key_track = 1.0f;
            p.env_amount_oct = 4.0f;
            p.filter_env = (StAdsr){0.0f, 0.1f, 0.5f, 0.1f};
            p.velocity_to_cutoff_oct = 3.0f;
            p.drive = 1.0f;
            p.pitch_env_semis = 36.0f;
            p.pitch_env_time = 0.02f;
            p.glide_time = 0.03f;
            for (int i = 0; i < 2; i++) {
                p.lfo[i].shape = i ? ST_LFO_SAMPLE_HOLD : ST_LFO_SQUARE;
                p.lfo[i].rate_hz = i ? 40.0f : 13.0f;
                p.lfo[i].to_pitch_cents = 1200.0f;
                p.lfo[i].to_cutoff_oct = 6.0f;
                p.lfo[i].to_amp = 1.0f;
                p.lfo[i].to_pan = 1.0f;
                p.lfo[i].to_pulse_width = 0.45f;
            }
            StSynthVoice v;
            st_synth_start(&v, &p, rate, 24.0f, 1.0f, 99);
            StVoiceMod mod = {1.0f, 1.0f, 3.0f, 0.0f};
            float note = 24.0f;
            int jumps = 0;
            for (int i = 0; i < n; i += ST_BLOCK) {
                if (i % (int)(0.25f * rate) < ST_BLOCK) {
                    /* a new note and velocity (0.5, 1, 0.25, ...) */
                    note = note >= 108.0f ? 12.0f : note + 13.0f;
                    jumps++;
                    st_synth_legato(&v, note, 0.25f * (float)(1 + jumps % 4));
                }
                mod.cutoff_oct = (i / ST_BLOCK) % 2 ? 6.0f : -6.0f;
                mod.pitch_cents = (i / ST_BLOCK) % 7 ? 0.0f : 2400.0f;
                st_synth_render(&v, &mod, l + i, r + i,
                                n - i < ST_BLOCK ? n - i : ST_BLOCK);
            }
            int finite = 1;
            for (int i = 0; i < n; i++)
                finite &= isfinite(l[i]) && isfinite(r[i]);
            double pk = peak(l, 0, n), pr = peak(r, 0, n);
            if (pr > pk)
                pk = pr;
            check(finite && pk < 4.0,
                  "%-6s %5.0f Hz, res 1, drive 1, stack 7, noise: finite, "
                  "peak %.3f", names[mi], rate, pk);
            free(l);
            free(r);
        }

    /* Self-oscillating ladder at its lowest cutoff, then released: the
     * state decays to exactly zero rather than through denormals. */
    StSynthPatch p;
    plain_patch(&p, ST_WAVE_SAW);
    p.filter = ST_FILTER_LADDER;
    p.cutoff_hz = 20.0f;
    p.resonance = 1.0f;
    p.amp.release = 0.01f;
    StSynthVoice v;
    float l[256], r[256];
    st_synth_start(&v, &p, 48000.0f, 30.0f, 1.0f, 1);
    for (int i = 0; i < 400; i++) {
        memset(l, 0, sizeof l);
        memset(r, 0, sizeof r);
        st_synth_render(&v, NULL, l, r, 256);
    }
    st_synth_release(&v);
    int alive = 1;
    for (int i = 0; i < 2000 && alive; i++)
        alive = st_synth_render(&v, NULL, l, r, 256);
    check(!alive && isfinite(v.flt[0][3]),
          "ladder at 20 Hz, res 1: finite, ends after release");
}

/* ------------------------------------------------------------------ */
/* 11. Performance.                                                    */

static void perf(const char *name, const StSynthPatch *p)
{
    const float rate = 48000.0f;
    int n = (int)(rate * 20.0f);
    float *l = buf(n), *r = buf(n);
    StSynthVoice v;
    clock_t t0 = clock();
    st_synth_start(&v, p, rate, 45.0f, 1.0f, 1);
    render(&v, l, r, n, ST_BLOCK);
    double secs = (double)(clock() - t0) / CLOCKS_PER_SEC;
    double fps = n / (secs > 0.0 ? secs : 1e-9);
    printf("        %-34s %6.2f Mframes/s = %5.0f voices in real time "
           "at 48 kHz\n", name, fps / 1e6, fps / rate);
    check(fps > rate, "%s renders faster than real time", name);
    free(l);
    free(r);
}

static void test_perf(void)
{
    printf("11. performance (one core, -O2)\n");
    StSynthPatch p;
    plain_patch(&p, ST_WAVE_SAW);
    p.filter = ST_FILTER_LP;
    p.cutoff_hz = 1500.0f;
    p.resonance = 0.3f;
    perf("(a) stack 1 saw + SVF", &p);
    plain_patch(&p, ST_WAVE_SAW);
    p.stack = 7;
    p.detune_cents = 25.0f;
    p.stereo_spread = 1.0f;
    p.sub_level = 0.3f;
    p.filter = ST_FILTER_LADDER;
    p.cutoff_hz = 1200.0f;
    p.resonance = 0.4f;
    p.drive = 0.2f;
    p.lfo[0].to_pitch_cents = 8.0f;
    p.lfo[1].to_cutoff_oct = 0.5f;
    perf("(b) stack 7 stereo ladder pad", &p);
}

int main(void)
{
    test_pitch();
    test_alias();
    test_filters();
    test_envelopes();
    test_filter_env();
    test_stack();
    test_glide();
    test_vibrato();
    test_determinism();
    test_stress();
    test_perf();
    printf("%d checks, %d failed\n", checks, failures);
    return failures ? 1 : 0;
}
