/*
 * tests/test_fx.c: checks for studio_fx (BRYX addition; see studio.h).
 *
 *   cd audio/engine && cc -std=c99 -O2 -Wall -Wextra -Werror \
 *       -o /tmp/test_fx tests/test_fx.c studio_fx.c studio_common.c -lm \
 *       && /tmp/test_fx
 *
 * Prints one line per check and exits non-zero if any fails.
 */
#include <math.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "../studio_fx.h"

static int failures = 0;
static int checks = 0;

static void check(int ok, const char *fmt, ...)
{
    va_list ap;
    checks++;
    if (!ok) failures++;
    printf("%s ", ok ? "PASS" : "FAIL");
    va_start(ap, fmt);
    vprintf(fmt, ap);
    va_end(ap);
    printf("\n");
}

static float *zalloc(size_t n)
{
    float *p = calloc(n, sizeof(float));
    if (!p) {
        fprintf(stderr, "out of memory\n");
        exit(2);
    }
    return p;
}

static double db(double x)
{
    return 20.0 * log10(x > 1e-30 ? x : 1e-30);
}

static double energy(const float *x, int a, int b)
{
    double e = 0.0;
    for (int i = a; i < b; i++) e += (double)x[i] * x[i];
    return e;
}

static double peak_abs(const float *x, int a, int b)
{
    double m = 0.0;
    for (int i = a; i < b; i++)
        if (fabs(x[i]) > m) m = fabs(x[i]);
    return m;
}

static const char *presets[] = { "room", "stage", "hall", "long" };

/* ------------------------------------------------------------------ */
/* Reverb helpers.                                                     */

/* Impulse response, an impulse on both inputs, in blocks of `block`. */
static void reverb_ir(const StReverbParams *p, float rate, int n, int block,
                      float *out_l, float *out_r)
{
    StReverb rv;
    float *in_l = zalloc((size_t)n), *in_r = zalloc((size_t)n);
    in_l[0] = in_r[0] = 1.0f;
    memset(out_l, 0, (size_t)n * sizeof(float));
    memset(out_r, 0, (size_t)n * sizeof(float));
    if (st_reverb_init(&rv, rate, p) != 0) {
        fprintf(stderr, "reverb init failed\n");
        exit(2);
    }
    for (int i = 0; i < n; i += block) {
        int m = n - i < block ? n - i : block;
        st_reverb_process(&rv, in_l + i, in_r + i, out_l + i, out_r + i, m);
    }
    st_reverb_free(&rv);
    free(in_l);
    free(in_r);
}

/* Reverberation time by Schroeder backward integration (M. R.
 * Schroeder, "New method of measuring reverberation time", JASA 37,
 * 1965): a least-squares line through the energy decay curve between
 * -5 and -35 dB (T30), extrapolated to -60 dB (ISO 3382-1). */
static double rt60_t30(const float *x, int n, float rate)
{
    double *edc = malloc((size_t)n * sizeof(double));
    double acc = 0.0, sx = 0, sy = 0, sxx = 0, sxy = 0, slope;
    int cnt = 0;
    for (int i = n - 1; i >= 0; i--) {
        acc += (double)x[i] * x[i];
        edc[i] = acc;
    }
    for (int i = 0; i < n; i++) {
        double l = 10.0 * log10(edc[i] / edc[0] + 1e-300);
        if (l <= -5.0 && l >= -35.0) {
            double t = (double)i / rate;
            sx += t; sy += l; sxx += t * t; sxy += t * l;
            cnt++;
        }
        if (l < -35.0) break;
    }
    free(edc);
    if (cnt < 10) return -1.0;
    slope = (cnt * sxy - sx * sy) / (cnt * sxx - sx * sx);
    return -60.0 / slope;
}

/* An octave band (4th-order: two high-pass and two low-pass biquads). */
static void octave_band(const float *x, float *y, int n, float rate, float fc)
{
    StBiquad f[4];
    memcpy(y, x, (size_t)n * sizeof(float));
    st_biquad_highpass(&f[0], rate, fc / sqrtf(2.0f), 0.7071f);
    st_biquad_highpass(&f[1], rate, fc / sqrtf(2.0f), 0.7071f);
    st_biquad_lowpass(&f[2], rate, fc * sqrtf(2.0f), 0.7071f);
    st_biquad_lowpass(&f[3], rate, fc * sqrtf(2.0f), 0.7071f);
    for (int k = 0; k < 4; k++) {
        st_biquad_reset(&f[k]);
        st_biquad_process(&f[k], y, NULL, n);
    }
}

/* ------------------------------------------------------------------ */

static void test_reverb_rt60(void)
{
    const float rates[2] = { 48000.0f, 44100.0f };
    printf("\n-- 1. reverb decay (T30, mean of the 500 Hz and 1 kHz octave"
           " bands; broadband for reference)\n");
    for (int ri = 0; ri < 2; ri++) {
        float rate = rates[ri];
        for (int k = 0; k < 4; k++) {
            StReverbParams p;
            int n;
            float *l, *r, *band;
            double t500, t1k, tmid, tbb, e;
            st_reverb_preset(presets[k], &p);
            n = (int)((1.5f * p.rt60 + 1.0f) * rate);
            l = zalloc((size_t)n);
            r = zalloc((size_t)n);
            band = zalloc((size_t)n);
            reverb_ir(&p, rate, n, 512, l, r);
            octave_band(l, band, n, rate, 500.0f);
            t500 = rt60_t30(band, n, rate);
            octave_band(l, band, n, rate, 1000.0f);
            t1k = rt60_t30(band, n, rate);
            tmid = 0.5 * (t500 + t1k);
            tbb = rt60_t30(l, n, rate);
            e = energy(l, 0, n);
            check(fabs(tmid - p.rt60) <= 0.15 * p.rt60,
                  "%-5s @ %5.0f Hz: RT60 %.3f s (500 Hz %.3f, 1 kHz %.3f, "
                  "broadband %.3f) vs %.2f s (%+.1f%%); IR energy %.1f dB",
                  presets[k], rate, tmid, t500, t1k, tbb, p.rt60,
                  100.0 * (tmid - p.rt60) / p.rt60, 10.0 * log10(e));
            free(l);
            free(r);
            free(band);
        }
    }
}

static void test_reverb_stereo(void)
{
    const float rate = 48000.0f;
    printf("\n-- 2. reverb stereo\n");
    for (int k = 0; k < 4; k++) {
        StReverbParams p;
        int n = (int)(2.0f * rate), a = (int)(0.05f * rate);
        float *l = zalloc((size_t)n), *r = zalloc((size_t)n);
        double sxy = 0, sxx = 0, syy = 0, corr;
        st_reverb_preset(presets[k], &p);
        p.width = 1.0f;
        reverb_ir(&p, rate, n, 256, l, r);
        for (int i = a; i < n; i++) {
            sxy += (double)l[i] * r[i];
            sxx += (double)l[i] * l[i];
            syy += (double)r[i] * r[i];
        }
        corr = sxy / sqrt(sxx * syy);
        check(fabs(corr) < 0.4, "%-5s width 1: L/R correlation after 50 ms = %+.3f",
              presets[k], corr);

        p.width = 0.0f;
        reverb_ir(&p, rate, n, 256, l, r);
        check(memcmp(l, r, (size_t)n * sizeof(float)) == 0 && energy(l, 0, n) > 0.0,
              "%-5s width 0: L and R identical", presets[k]);
        free(l);
        free(r);
    }
}

static void test_reverb_stability(void)
{
    const float rate = 48000.0f;
    printf("\n-- 3. reverb stability (30 s of full-scale noise, then silence)\n");
    for (int k = 0; k < 4; k++) {
        StReverbParams p;
        StReverb rv;
        int noise_n = (int)(30.0f * rate), n, bad = 0;
        uint32_t seed = 12345u + (uint32_t)k;
        float *in_l, *in_r, *l, *r;
        double rms_noise, mean = 0.0, rms_tail;
        int a, tail_a;
        st_reverb_preset(presets[k], &p);
        n = noise_n + (int)((3.0f * p.rt60 + 1.0f) * rate);
        in_l = zalloc((size_t)n);
        in_r = zalloc((size_t)n);
        l = zalloc((size_t)n);
        r = zalloc((size_t)n);
        for (int i = 0; i < noise_n; i++) {
            in_l[i] = st_randf(&seed);
            in_r[i] = st_randf(&seed);
        }
        st_reverb_init(&rv, rate, &p);
        for (int i = 0; i < n; i += 1024) {
            int m = n - i < 1024 ? n - i : 1024;
            st_reverb_process(&rv, in_l + i, in_r + i, l + i, r + i, m);
        }
        st_reverb_free(&rv);
        for (int i = 0; i < n; i++)
            if (!isfinite(l[i]) || !isfinite(r[i])) bad++;
        a = noise_n - (int)(10.0f * rate);
        rms_noise = sqrt(energy(l, a, noise_n) / (noise_n - a));
        for (int i = a; i < noise_n; i++) mean += l[i] + r[i];
        mean /= 2.0 * (noise_n - a);
        tail_a = n - (int)(0.5f * rate);
        rms_tail = sqrt((energy(l, tail_a, n) + energy(r, tail_a, n)) / (2.0 * (n - tail_a)));
        check(bad == 0 && rms_tail < rms_noise * 1e-6 && fabs(mean) < 0.01 * rms_noise,
              "%-5s: %d non-finite; wet RMS %.1f dB; DC %.1f dB below it; "
              "after 3 x RT60 of silence %.1f dB",
              presets[k], bad, db(rms_noise), db(rms_noise) - db(fabs(mean)),
              db(rms_tail));
        free(in_l);
        free(in_r);
        free(l);
        free(r);
    }
}

static void test_reverb_predelay(void)
{
    const float rate = 48000.0f;
    printf("\n-- 4. reverb pre-delay\n");
    for (int k = 0; k < 4; k++) {
        StReverbParams p;
        int n = (int)(0.3f * rate), first = -1;
        float *l = zalloc((size_t)n), *r = zalloc((size_t)n);
        double pk, at;
        st_reverb_preset(presets[k], &p);
        reverb_ir(&p, rate, n, 128, l, r);
        pk = peak_abs(l, 0, n);
        if (peak_abs(r, 0, n) > pk) pk = peak_abs(r, 0, n);
        for (int i = 0; i < n && first < 0; i++)
            if (fabs(l[i]) > 1e-3 * pk || fabs(r[i]) > 1e-3 * pk) first = i;
        at = (double)first / rate;
        check(fabs(at - p.predelay) <= 0.001,
              "%-5s: first arrival %.2f ms (pre-delay %.1f ms)", presets[k],
              1000.0 * at, 1000.0 * p.predelay);
        free(l);
        free(r);
    }
}

/* ------------------------------------------------------------------ */
/* Delay.                                                              */

static int first_above(const float *x, int a, int n, double thr)
{
    for (int i = a; i < n; i++)
        if (fabs(x[i]) > thr) return i;
    return -1;
}

static int argmax_abs(const float *x, int a, int b)
{
    int at = a;
    for (int i = a; i < b; i++)
        if (fabs(x[i]) > fabs(x[at])) at = i;
    return at;
}

static void run_delay(const StDelayParams *p, float rate, int n, int click_l,
                      int click_r, float *l, float *r)
{
    StDelay d;
    float *in_l = zalloc((size_t)n), *in_r = zalloc((size_t)n);
    in_l[0] = (float)click_l;
    in_r[0] = (float)click_r;
    memset(l, 0, (size_t)n * sizeof(float));
    memset(r, 0, (size_t)n * sizeof(float));
    if (st_delay_init(&d, rate, p, 2.0f) != 0) {
        fprintf(stderr, "delay init failed\n");
        exit(2);
    }
    for (int i = 0; i < n; i += 300) {
        int m = n - i < 300 ? n - i : 300;
        st_delay_process(&d, in_l + i, in_r + i, l + i, r + i, m);
    }
    st_delay_free(&d);
    free(in_l);
    free(in_r);
}

/* High-frequency share of a stretch: energy of the first difference over
 * energy (a brightness proxy, 0..4). */
static double brightness(const float *x, int a, int b)
{
    double e = 0.0, de = 0.0;
    for (int i = a + 1; i < b; i++) {
        double dlt = (double)x[i] - x[i - 1];
        e += (double)x[i] * x[i];
        de += dlt * dlt;
    }
    return de / (e + 1e-30);
}

static void test_delay(void)
{
    printf("\n-- 5. delay\n");
    {
        const float rates[2] = { 48000.0f, 44100.0f };
        for (int ri = 0; ri < 2; ri++) {
            float rate = rates[ri];
            StDelayParams p = { 0.375f, 0.25f, 0.5f, 0, 0.0f, 0.0f };
            int n = (int)(1.6f * rate), il, ir, pl, pr;
            float *l = zalloc((size_t)n), *r = zalloc((size_t)n);
            double expect_l = 0.375 * rate, expect_r = 0.25 * rate;
            run_delay(&p, rate, n, 1, 1, l, r);
            il = first_above(l, 0, n, 0.05);
            ir = first_above(r, 0, n, 0.05);
            pl = argmax_abs(l, 0, (int)(0.5f * rate));
            pr = argmax_abs(r, 0, (int)(0.3f * rate));
            check(fabs(pl - expect_l) <= 1.0 && fabs(pr - expect_r) <= 1.0 &&
                      il >= expect_l - 2 && ir >= expect_r - 2,
                  "stereo @ %5.0f Hz: first repeat L at frame %d (expect %.1f), "
                  "R at %d (expect %.1f)", rate, pl, expect_l, pr, expect_r);
            if (ri == 0) {
                /* repeats of L at 0.375 s spacing: -6 dB apart */
                int w = 16, s = 18000;
                double e1 = energy(l, s - w, s + w), e2 = energy(l, 2 * s - w, 2 * s + w),
                       e3 = energy(l, 3 * s - w, 3 * s + w);
                double d12 = 10.0 * log10(e2 / e1), d23 = 10.0 * log10(e3 / e2);
                check(fabs(d12 + 6.02) <= 1.0 && fabs(d23 + 6.02) <= 1.0,
                      "feedback 0.5, filters open: repeats %+.2f dB, %+.2f dB apart",
                      d12, d23);
            }
            free(l);
            free(r);
        }
    }
    {
        const float rate = 48000.0f;
        StDelayParams p = { 0.375f, 0.375f, 0.5f, 1, 0.0f, 0.0f };
        int n = (int)(1.3f * rate), s = 18000, w = 8;
        float *l = zalloc((size_t)n), *r = zalloc((size_t)n);
        double l1, r1, l2, r2, l3, r3;
        run_delay(&p, rate, n, 1, 1, l, r);
        l1 = peak_abs(l, s - w, s + w); r1 = peak_abs(r, s - w, s + w);
        l2 = peak_abs(l, 2 * s - w, 2 * s + w); r2 = peak_abs(r, 2 * s - w, 2 * s + w);
        l3 = peak_abs(l, 3 * s - w, 3 * s + w); r3 = peak_abs(r, 3 * s - w, 3 * s + w);
        check(first_above(l, 0, n, 0.05) == s && l1 > 0.99 && r1 < 1e-6 &&
                  r2 > 0.49 && l2 < 1e-6 && l3 > 0.24 && r3 < 1e-6,
              "ping-pong: repeat 1 L %.3f / R %.3f, repeat 2 L %.3f / R %.3f, "
              "repeat 3 L %.3f / R %.3f", l1, r1, l2, r2, l3, r3);
        free(l);
        free(r);
    }
    {
        const float rate = 48000.0f;
        StDelayParams p = { 0.375f, 0.375f, 0.5f, 0, 250.0f, 2500.0f };
        int n = (int)(1.6f * rate), s = 18000, w = 3000;
        float *l = zalloc((size_t)n), *r = zalloc((size_t)n);
        double b[3], e[3];
        run_delay(&p, rate, n, 1, 1, l, r);
        for (int k = 0; k < 3; k++) {
            b[k] = brightness(l, (k + 1) * s - 4, (k + 1) * s + w);
            e[k] = energy(l, (k + 1) * s - 4, (k + 1) * s + w);
        }
        check(b[1] < b[0] && b[2] < b[1] && e[1] / e[0] < 0.25 && e[2] / e[1] < 0.25,
              "filters 250 Hz..2.5 kHz: brightness %.4f > %.4f > %.4f; repeats "
              "%+.1f, %+.1f dB apart", b[0], b[1], b[2], 10.0 * log10(e[1] / e[0]),
              10.0 * log10(e[2] / e[1]));
        free(l);
        free(r);
    }
    {
        /* a tempo ramp: retime every block from 0.375 s to 0.30 s while a
         * 220 Hz sine plays; the repeats must glide, never jump */
        const float rate = 48000.0f;
        StDelayParams p = { 0.375f, 0.375f, 0.4f, 1, 0.0f, 0.0f };
        StDelay d;
        int n = 3 * 48000;
        float *in = zalloc((size_t)n), *l = zalloc((size_t)n), *r = zalloc((size_t)n);
        double jump = 0.0, max_step;
        for (int i = 0; i < n; i++)
            in[i] = (float)(0.5 * sin(2.0 * 3.14159265358979323846 * 220.0 * i / rate));
        st_delay_init(&d, rate, &p, 1.0f);
        for (int i = 0; i < n; i += ST_BLOCK) {
            if (i > 48000 && i < 96000) {
                p.time_l = p.time_r = 0.375f - 0.075f * (float)(i - 48000) / 48000.0f;
                st_delay_set_params(&d, &p);
            }
            st_delay_process(&d, in + i, in + i, l + i, r + i, ST_BLOCK);
        }
        st_delay_free(&d);
        for (int i = 1; i < n; i++) {
            double s = fabs((double)l[i] - l[i - 1]);
            if (s > jump) jump = s;
        }
        /* a 220 Hz sine of the repeats' peak amplitude moves at most
         * 2 pi 220 / 48000 * peak per frame; allow the pitch bend */
        max_step = 2.0 * 3.14159265358979323846 * 220.0 / rate * peak_abs(l, 0, n) * 1.5;
        check(jump <= max_step, "tempo ramp 0.375 -> 0.300 s: largest step %.4f "
              "(smooth bound %.4f)", jump, max_step);
        free(in);
        free(l);
        free(r);
    }
}

/* ------------------------------------------------------------------ */
/* Biquads.                                                            */

static double sine_gain_db(StBiquad *b, float rate, float hz)
{
    int n = (int)(2.0f * rate), a = (int)(1.0f * rate);
    float *x = zalloc((size_t)n), *y = zalloc((size_t)n);
    double g;
    for (int i = 0; i < n; i++)
        x[i] = y[i] = (float)sin(2.0 * 3.14159265358979323846 * hz * i / rate);
    st_biquad_reset(b);
    st_biquad_process(b, y, NULL, n);
    g = 10.0 * log10(energy(y, a, n) / energy(x, a, n));
    free(x);
    free(y);
    return g;
}

static void test_biquads(void)
{
    const float rates[2] = { 44100.0f, 48000.0f };
    printf("\n-- 6. biquads (sine at the design frequency)\n");
    for (int ri = 0; ri < 2; ri++) {
        float rate = rates[ri];
        StBiquad b;
        double g;
        struct { const char *name; float hz; double expect; } cases[] = {
            { "lowpass   1 kHz q .7071", 1000.0f, -3.0103 },
            { "lowpass  10 kHz q .7071", 10000.0f, -3.0103 },
            { "highpass 1 kHz q .7071", 1000.0f, -3.0103 },
            { "highpass 40 Hz q .7071", 40.0f, -3.0103 },
            { "peak 2 kHz q 1.4 +6 dB", 2000.0f, 6.0 },
            { "peak 300 Hz q 0.7 -9 dB", 300.0f, -9.0 },
            { "lowshelf 200 Hz +6 dB", 200.0f, 3.0 },
            { "highshelf 6 kHz -8 dB", 6000.0f, -4.0 },
        };
        for (int c = 0; c < 8; c++) {
            switch (c) {
            case 0: case 1: st_biquad_lowpass(&b, rate, cases[c].hz, 0.7071f); break;
            case 2: case 3: st_biquad_highpass(&b, rate, cases[c].hz, 0.7071f); break;
            case 4: st_biquad_peak(&b, rate, cases[c].hz, 1.4f, 6.0f); break;
            case 5: st_biquad_peak(&b, rate, cases[c].hz, 0.7f, -9.0f); break;
            case 6: st_biquad_lowshelf(&b, rate, cases[c].hz, 6.0f); break;
            default: st_biquad_highshelf(&b, rate, cases[c].hz, -8.0f); break;
            }
            g = sine_gain_db(&b, rate, cases[c].hz);
            check(fabs(g - cases[c].expect) <= 0.3, "%5.0f Hz %-26s %+.3f dB (design %+.3f)",
                  rate, cases[c].name, g, cases[c].expect);
        }
    }
}

/* ------------------------------------------------------------------ */
/* Compressor.                                                         */

static void test_comp(void)
{
    const float rate = 48000.0f;
    const double amp = pow(10.0, -10.0 / 20.0), w = 2.0 * 3.14159265358979323846 * 1000.0 / rate;
    StCompParams p = { -20.0f, 4.0f, 0.010f, 0.100f, 0.0f, 0.0f };
    StComp c;
    int n = (int)(2.0f * rate), a = (int)(1.5f * rate), t63 = -1;
    float *l = zalloc((size_t)n), *r = zalloc((size_t)n);
    double out_db, gr;
    printf("\n-- 7. compressor\n");
    for (int i = 0; i < n; i++) l[i] = r[i] = (float)(amp * sin(w * i));
    st_comp_init(&c, rate, &p);
    /* sample by sample, to time the attack */
    for (int i = 0; i < n; i++) {
        st_comp_process(&c, l + i, r + i, NULL, NULL, 1);
        if (t63 < 0 && st_comp_gain_reduction_db(&c) >= 0.632 * 7.5) t63 = i;
    }
    out_db = db(peak_abs(l, a, n));
    gr = st_comp_gain_reduction_db(&c);
    check(fabs(out_db + 17.5) <= 0.5 && memcmp(l, r, (size_t)n * sizeof(float)) == 0,
          "sine -10 dBFS, threshold -20, ratio 4, hard knee: out %.2f dBFS "
          "(expect -17.5), gain reduction %.2f dB", out_db, gr);
    check(t63 > 0 && t63 / rate > 0.5 * p.attack && t63 / rate < 2.5 * p.attack,
          "attack 10 ms: gain reduction reaches 63%% after %.2f ms", 1000.0 * t63 / rate);

    /* release: the sine stops; 63% recovery after ~release */
    {
        int t = -1;
        float z[1] = { 0.0f }, z2[1] = { 0.0f };
        double start = st_comp_gain_reduction_db(&c);
        for (int i = 0; i < (int)(rate); i++) {
            z[0] = z2[0] = 0.0f;
            st_comp_process(&c, z, z2, NULL, NULL, 1);
            if (t < 0 && st_comp_gain_reduction_db(&c) <= start * 0.368) t = i;
        }
        check(t > 0 && fabs(t / rate - p.release) < 0.2 * p.release,
              "release 100 ms: 63%% recovered after %.1f ms", 1000.0 * t / rate);
    }

    /* soft knee: continuous and between the hard curve and no compression */
    {
        StCompParams pk = p;
        float x[1], y[1];
        int ok = 1;
        double prev = -1e9;
        pk.knee_db = 10.0f;
        pk.attack = pk.release = 0.0f;
        for (int k = 0; k <= 400; k++) {
            double in_db = -40.0 + 0.1 * k, o;
            st_comp_init(&c, rate, &pk);
            x[0] = y[0] = (float)pow(10.0, in_db / 20.0);
            st_comp_process(&c, x, y, NULL, NULL, 1);
            o = db(x[0]);
            if (o < prev - 1e-6) ok = 0;            /* monotonic */
            if (o > in_db + 1e-4) ok = 0;           /* never gains */
            if (in_db > -15.0 && fabs(o - (-20.0 + (in_db + 20.0) / 4.0)) > 1e-3) ok = 0;
            if (in_db < -25.0 && fabs(o - in_db) > 1e-3) ok = 0;
            prev = o;
        }
        check(ok, "10 dB soft knee: monotonic, exact outside the knee");
    }

    /* external key: a loud key ducks a quiet signal */
    {
        float sl[4800], sr[4800], kl[4800], kr[4800];
        StCompParams ps = { -30.0f, 10.0f, 0.001f, 0.1f, 0.0f, 0.0f };
        double g;
        for (int i = 0; i < 4800; i++) {
            sl[i] = sr[i] = 0.01f;     /* -40 dBFS: below threshold */
            kl[i] = kr[i] = 0.5f;      /* -6 dBFS key */
        }
        st_comp_init(&c, rate, &ps);
        st_comp_process(&c, sl, sr, kl, kr, 4800);
        g = db(sl[4799] / 0.01);
        check(fabs(g + 21.6) < 0.2, "sidechain: -6 dBFS key ducks a -40 dBFS signal by "
              "%.2f dB (expect -21.6)", g);
    }
    free(l);
    free(r);
}

/* ------------------------------------------------------------------ */
/* Limiter.                                                            */

static void test_limiter(void)
{
    const float rate = 48000.0f;
    const float ceiling_db = -1.0f, ceiling = st_db_to_gain(ceiling_db);
    StLimiter lm;
    int n = (int)(10.0f * rate);
    float *l = zalloc((size_t)n), *r = zalloc((size_t)n);
    float *xl = zalloc((size_t)n), *xr = zalloc((size_t)n);
    uint32_t seed = 777u;
    double in_peak, out_peak;
    int lat;
    printf("\n-- 8. limiter (ceiling -1 dBFS, lookahead 5 ms, release 80 ms)\n");

    /* program: a -6 dBFS sine bed, noise bursts and single-sample clicks
     * peaking +12 dBFS */
    for (int i = 0; i < n; i++) {
        double t = (double)i / rate;
        l[i] = (float)(0.5 * sin(2.0 * 3.14159265358979323846 * 110.0 * t));
        r[i] = (float)(0.5 * sin(2.0 * 3.14159265358979323846 * 165.0 * t));
        if ((i % 9600) < 240) {
            l[i] += 3.98f * st_randf(&seed);
            r[i] += 3.98f * st_randf(&seed);
        }
        if (i % 4801 == 2400) l[i] = (i & 1) ? 3.98f : -3.98f;
    }
    in_peak = peak_abs(l, 0, n);
    st_limiter_init(&lm, rate, ceiling_db, 0.005f, 0.080f);
    lat = st_limiter_latency(&lm);
    for (int i = 0; i < n; i += 512) {
        int m = n - i < 512 ? n - i : 512;
        st_limiter_process(&lm, l + i, r + i, m);
    }
    out_peak = peak_abs(l, 0, n);
    if (peak_abs(r, 0, n) > out_peak) out_peak = peak_abs(r, 0, n);
    check(out_peak <= ceiling && lm.clamps == 0,
          "peaks %+.1f dBFS in -> %.4f dBFS out (ceiling %.1f), %lld clamps",
          db(in_peak), db(out_peak), ceiling_db, lm.clamps);
    check(lat == 240, "latency %d frames (5 ms at 48 kHz = 240)", lat);
    st_limiter_free(&lm);

    /* below the ceiling: the input, delayed, unchanged */
    {
        double worst = 0.0;
        for (int i = 0; i < n; i++) {
            double t = (double)i / rate;
            xl[i] = l[i] = (float)(0.85 * sin(2.0 * 3.14159265358979323846 * 440.0 * t));
            xr[i] = r[i] = (float)(0.6 * st_randf(&seed));
        }
        st_limiter_init(&lm, rate, ceiling_db, 0.005f, 0.080f);
        st_limiter_process(&lm, l, r, n);
        for (int i = lat; i < n; i++) {
            double el = fabs((double)l[i] - xl[i - lat]), er = fabs((double)r[i] - xr[i - lat]);
            if (el > worst) worst = el;
            if (er > worst) worst = er;
        }
        check(worst <= 0.0012 * 0.85 && peak_abs(l, 0, lat) == 0.0,
              "below the ceiling: output = input delayed %d frames, worst error %.3g "
              "(0.01 dB of 0.85 = %.3g)", lat, worst, 0.85 * (pow(10.0, 0.01 / 20.0) - 1.0));
        st_limiter_free(&lm);
    }

    /* a full-scale square wave at +12 dB: the worst case for overshoot */
    {
        for (int i = 0; i < n; i++) {
            l[i] = ((i / 37) & 1) ? 3.98f : -3.98f;
            r[i] = -l[i];
        }
        st_limiter_init(&lm, rate, ceiling_db, 0.002f, 0.010f);
        st_limiter_process(&lm, l, r, n);
        out_peak = peak_abs(l, 0, n);
        check(out_peak <= ceiling && lm.clamps == 0,
              "+12 dB square, 2 ms lookahead, 10 ms release: %.4f dBFS, %lld clamps",
              db(out_peak), lm.clamps);
        st_limiter_free(&lm);
    }
    free(l);
    free(r);
    free(xl);
    free(xr);
}

/* ------------------------------------------------------------------ */
/* Determinism.                                                        */

static void render_chain(int block, float *l, float *r, int n, const float *in_l,
                         const float *in_r)
{
    StReverb rv;
    StDelay d;
    StComp c;
    StLimiter lm;
    StBiquad eq;
    StReverbParams p;
    StDelayParams dp = { 0.31f, 0.47f, 0.6f, 1, 150.0f, 5000.0f };
    StCompParams cp = { -18.0f, 3.0f, 0.01f, 0.2f, 6.0f, 2.0f };
    float *a = zalloc((size_t)n), *b = zalloc((size_t)n);
    st_reverb_preset("hall", &p);
    st_reverb_init(&rv, 48000.0f, &p);
    st_delay_init(&d, 48000.0f, &dp, 1.0f);
    st_comp_init(&c, 48000.0f, &cp);
    st_limiter_init(&lm, 48000.0f, -1.0f, 0.005f, 0.1f);
    st_biquad_highshelf(&eq, 48000.0f, 3000.0f, 3.0f);
    st_biquad_reset(&eq);
    memcpy(l, in_l, (size_t)n * sizeof(float));
    memcpy(r, in_r, (size_t)n * sizeof(float));
    for (int i = 0; i < n; i += block) {
        int m = n - i < block ? n - i : block;
        memset(a + i, 0, (size_t)m * sizeof(float));
        memset(b + i, 0, (size_t)m * sizeof(float));
        st_reverb_process(&rv, in_l + i, in_r + i, a + i, b + i, m);
        st_delay_process(&d, in_l + i, in_r + i, a + i, b + i, m);
        for (int k = i; k < i + m; k++) {
            l[k] += 0.5f * a[k];
            r[k] += 0.5f * b[k];
        }
        st_biquad_process(&eq, l + i, r + i, m);
        st_comp_process(&c, l + i, r + i, NULL, NULL, m);
        st_limiter_process(&lm, l + i, r + i, m);
    }
    st_reverb_free(&rv);
    st_delay_free(&d);
    st_limiter_free(&lm);
    free(a);
    free(b);
}

static void test_determinism(void)
{
    int n = 5 * 48000;
    uint32_t seed = 4242u;
    float *in_l = zalloc((size_t)n), *in_r = zalloc((size_t)n);
    float *l1 = zalloc((size_t)n), *r1 = zalloc((size_t)n);
    float *l2 = zalloc((size_t)n), *r2 = zalloc((size_t)n);
    float *l3 = zalloc((size_t)n), *r3 = zalloc((size_t)n);
    printf("\n-- 9. determinism (reverb + delay + EQ + compressor + limiter)\n");
    for (int i = 0; i < n; i++) {
        float env = (i / 6000) % 3 == 0 ? 1.0f : 0.1f;
        in_l[i] = env * st_randf(&seed);
        in_r[i] = env * st_randf(&seed);
    }
    render_chain(ST_BLOCK, l1, r1, n, in_l, in_r);
    render_chain(ST_BLOCK, l2, r2, n, in_l, in_r);
    render_chain(1, l3, r3, n, in_l, in_r);
    check(memcmp(l1, l2, (size_t)n * sizeof(float)) == 0 &&
              memcmp(r1, r2, (size_t)n * sizeof(float)) == 0,
          "same input, same output bytes");
    check(memcmp(l1, l3, (size_t)n * sizeof(float)) == 0 &&
              memcmp(r1, r3, (size_t)n * sizeof(float)) == 0,
          "block size 1 and %d render the same bytes", ST_BLOCK);
    free(in_l); free(in_r); free(l1); free(r1); free(l2); free(r2); free(l3); free(r3);
}

/* ------------------------------------------------------------------ */
/* Performance.                                                        */

static void test_performance(void)
{
    const float rate = 48000.0f;
    int n = 20 * 48000;
    uint32_t seed = 99u;
    float *in_l = zalloc((size_t)n), *in_r = zalloc((size_t)n);
    float *l = zalloc((size_t)n), *r = zalloc((size_t)n);
    clock_t t0;
    double secs;
    printf("\n-- 10. performance (20 s of stereo noise at 48 kHz, blocks of %d)\n", ST_BLOCK);
    for (int i = 0; i < n; i++) {
        in_l[i] = st_randf(&seed);
        in_r[i] = st_randf(&seed);
    }
    {
        StReverb rv;
        StReverbParams p;
        st_reverb_preset("hall", &p);
        st_reverb_init(&rv, rate, &p);
        t0 = clock();
        for (int i = 0; i < n; i += ST_BLOCK)
            st_reverb_process(&rv, in_l + i, in_r + i, l + i, r + i, ST_BLOCK);
        secs = (double)(clock() - t0) / CLOCKS_PER_SEC;
        st_reverb_free(&rv);
        check(secs > 0.0 && n / secs > 48000.0,
              "hall reverb: %.2f M frames/s (%.0fx real time)", n / secs / 1e6,
              n / secs / rate);
    }
    {
        StDelay d;
        StDelayParams p = { 0.375f, 0.25f, 0.5f, 1, 200.0f, 6000.0f };
        st_delay_init(&d, rate, &p, 2.0f);
        t0 = clock();
        for (int i = 0; i < n; i += ST_BLOCK)
            st_delay_process(&d, in_l + i, in_r + i, l + i, r + i, ST_BLOCK);
        secs = (double)(clock() - t0) / CLOCKS_PER_SEC;
        st_delay_free(&d);
        check(secs > 0.0 && n / secs > 48000.0, "ping-pong delay: %.2f M frames/s",
              n / secs / 1e6);
    }
    {
        StComp c;
        StCompParams p = { -18.0f, 3.0f, 0.01f, 0.2f, 6.0f, 2.0f };
        memcpy(l, in_l, (size_t)n * sizeof(float));
        memcpy(r, in_r, (size_t)n * sizeof(float));
        st_comp_init(&c, rate, &p);
        t0 = clock();
        for (int i = 0; i < n; i += ST_BLOCK)
            st_comp_process(&c, l + i, r + i, NULL, NULL, ST_BLOCK);
        secs = (double)(clock() - t0) / CLOCKS_PER_SEC;
        check(secs > 0.0 && n / secs > 48000.0, "compressor: %.2f M frames/s",
              n / secs / 1e6);
    }
    {
        StLimiter lm;
        for (int i = 0; i < n; i++) {
            l[i] = 4.0f * in_l[i];
            r[i] = 4.0f * in_r[i];
        }
        st_limiter_init(&lm, rate, -1.0f, 0.005f, 0.08f);
        t0 = clock();
        for (int i = 0; i < n; i += ST_BLOCK)
            st_limiter_process(&lm, l + i, r + i, ST_BLOCK);
        secs = (double)(clock() - t0) / CLOCKS_PER_SEC;
        check(secs > 0.0 && n / secs > 48000.0 && lm.clamps == 0,
              "limiter (5 ms, driven +12 dB): %.2f M frames/s, %lld clamps",
              n / secs / 1e6, lm.clamps);
        st_limiter_free(&lm);
    }
    free(in_l); free(in_r); free(l); free(r);
}

int main(void)
{
    StReverbParams p;
    printf("studio_fx tests\n");
    check(st_reverb_preset("hall", &p) == 1 && st_reverb_preset("cave", &p) == 0,
          "presets: hall known, cave unknown");
    test_reverb_rt60();
    test_reverb_stereo();
    test_reverb_stability();
    test_reverb_predelay();
    test_delay();
    test_biquads();
    test_comp();
    test_limiter();
    test_determinism();
    test_performance();
    printf("\n%d checks, %d failed\n", checks, failures);
    return failures ? 1 : 0;
}
