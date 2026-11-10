/*
 * studio.c: the .score parser, scheduler, mixer and render (BRYX
 * addition; see studio.h and docs/design/orchestral-score.md).
 *
 * A .score is line-oriented text. `#` at the start of a line or after a
 * space starts a comment (inside a word it is a sharp: F#3). Times are in
 * beats; the tempo map turns them into seconds.
 *
 *   score 1
 *   title <words>
 *   rate 48000                    render rate (optional)
 *   tempo <beat> <bpm>            tempo map: a new tempo from <beat> on
 *   length <beats>                the loop or the piece
 *   loop on                       render twice, keep the second pass: the
 *                                 seam carries the end's tails (reverb,
 *                                 delay, releases) into the start
 *   tail <seconds>                extra render after the end (no loop)
 *   seed <n>                      seed for every voice's randomness
 *   include <path>                another file (instrument libraries)
 *
 *   aux <name> reverb preset=<room|stage|hall|long> [rt60= predelay=
 *       size= damping= lowcut= width= mod=] [gain=<dB>]
 *   aux <name> delay time=<beats> [time_r=<beats>] feedback= [pingpong=1]
 *       [lowcut=<Hz>] [highcut=<Hz>] [gain=<dB>]
 *   bus <name> [gain=<dB>] [hpf=<Hz>] [lowshelf=<Hz>:<dB>]
 *       [highshelf=<Hz>:<dB>] [peak=<Hz>:<q>:<dB>]
 *       [comp=<threshold dB>:<ratio>:<attack s>:<release s>[:<makeup dB>]]
 *       [key=<instrument>] [send=<aux>:<dB>]...
 *   master [gain=<dB>] [comp=...] [limit=<ceiling dB>]
 *
 *   inst <name> sfz=<path> | synth   [bus=<name>] [gain=<dB>] [pan=-1..1]
 *       [send=<aux>:<dB>]... [poly=<voices>] then patch keys:
 *     sampler: attack= decay= sustain= release= velcurve= offset= tune=
 *              stack= stack_detune= stack_delay= spread=
 *     synth:   wave=saw|square|pulse|triangle|sine|noise pw= stack=
 *              detune= spread= sub= noise= attack= decay= sustain=
 *              release= filter=none|lp|hp|bp|ladder cutoff= res=
 *              keytrack= fenv=<octaves> fattack= fdecay= fsustain=
 *              frelease= velcutoff= drive= pitchenv=<semitones>
 *              pitchtime= glide= mono=1
 *              lfo1=<shape>:<rate Hz>:<delay s> lfo1_pitch=<cents>
 *              lfo1_cutoff=<oct> lfo1_amp= lfo1_pan= lfo1_pw= (lfo2_* too)
 *
 *   note <beat> <beats> <inst> <pitch> <velocity 1..127> [legato]
 *       pitch: a MIDI number (may be fractional) or a name (E4, F#3, Bb2)
 *   ramp <target>.<param> <beat0> <beat1> <from> <to> [lin|smooth]
 *       target: an instrument, a bus, an aux or `master`
 *       params: gain (dB, all), cutoff (octaves, instruments),
 *               pitch (cents, instruments)
 *       A lane holds its last value after a ramp ends.
 *
 * The signal path: voices -> instrument (gain, pan, sends) -> group bus
 * (EQ, compressor, gain, sends) -> master; aux returns (reverb, delay) take
 * the sends and feed the master; master gain, compressor and limiter.
 */
#include <ctype.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "studio.h"
#include "studio_fx.h"
#include "studio_sampler.h"
#include "studio_synth.h"

#define MAX_INST 48
#define MAX_BUS 8
#define MAX_AUX 6
#define MAX_SENDS 4
#define MAX_EQ 4
#define MAX_TEMPO 64
#define MAX_VOICES 256
#define MAX_DEPTH 4
#define DEFAULT_POLY 24

enum { KIND_SYNTH = 0, KIND_SAMPLER = 1 };
enum { T_INST = 0, T_BUS, T_AUX, T_MASTER };
enum { P_GAIN = 0, P_CUTOFF, P_PITCH, P_COUNT };

typedef struct { int aux; float gain; } Send;

typedef struct {
    char name[32];
    int kind;
    StSynthPatch synth;
    StSamplerPatch samp;
    StSampleSet *set;
    char sfz[512];
    int bus;
    float gain_db, pan;
    Send sends[MAX_SENDS];
    int nsend;
    int poly;
    unsigned rr;
} Inst;

typedef struct {
    int type;                /* 0 hpf, 1 lpf, 2 lowshelf, 3 highshelf, 4 peak */
    float hz, a, b;
} EqSpec;

typedef struct {
    char name[32];
    float gain_db;
    EqSpec eqspec[MAX_EQ];   /* built at render time, at the render's rate */
    StBiquad eq[MAX_EQ];
    int neq;
    int has_comp;
    StCompParams compp;
    StComp comp;
    int key;                 /* instrument index keying the compressor, -1 none */
    Send sends[MAX_SENDS];
    int nsend;
} Bus;

typedef struct {
    char name[32];
    int kind;                /* 0 reverb, 1 delay */
    StReverbParams rp;
    StDelayParams dp;
    double time_beats_l, time_beats_r;
    float gain_db;
    StReverb rv;
    StDelay dl;
    int ready;
} Aux;

typedef struct {
    double beat, dur;
    int inst;
    float pitch;
    int vel;
    int legato;
} Note;

typedef struct {
    int tkind, index, param;
    double b0, b1;
    float v0, v1;
    int smooth;
} Ramp;

typedef struct {
    long frame;
    int on;                  /* 1 note-on, 0 note-off */
    int note;                /* index into notes */
    int pass;
} Event;

typedef struct {
    int active;
    int inst, note, pass;
    int released;
    long started;
    StSynthVoice syn;
    StSamplerVoice smp;
} Voice;

typedef struct {
    int rate;
    double length;
    int loop;
    double tail;
    uint32_t seed;
    double tempo_beat[MAX_TEMPO], tempo_bpm[MAX_TEMPO];
    int ntempo;
    Inst inst[MAX_INST]; int ninst;
    Bus bus[MAX_BUS]; int nbus;
    Aux aux[MAX_AUX]; int naux;
    float master_gain_db;
    int master_has_comp;
    StCompParams master_compp;
    StComp master_comp;
    float limit_db;
    int has_limit;
    Note *notes; int nnotes, capnotes;
    Ramp *ramps; int nramps, capramps;
    char *err; size_t errlen;
} Score;

static int fail(Score *s, const char *file, int line, const char *msg, const char *arg)
{
    snprintf(s->err, s->errlen, "%s:%d: %s%s%s", file, line, msg, arg ? " " : "", arg ? arg : "");
    return -1;
}

/* ------------------------------------------------------------------ */
/* Parsing helpers                                                      */

static int find_inst(const Score *s, const char *n)
{
    for (int i = 0; i < s->ninst; i++)
        if (!strcmp(s->inst[i].name, n)) return i;
    return -1;
}
static int find_bus(const Score *s, const char *n)
{
    for (int i = 0; i < s->nbus; i++)
        if (!strcmp(s->bus[i].name, n)) return i;
    return -1;
}
static int find_aux(const Score *s, const char *n)
{
    for (int i = 0; i < s->naux; i++)
        if (!strcmp(s->aux[i].name, n)) return i;
    return -1;
}

/* "E4" -> 64, "F#3" -> 54, "Bb2" -> 46, "61.5" -> 61.5; -1000 on error. */
static float parse_pitch(const char *t)
{
    static const int base[7] = {9, 11, 0, 2, 4, 5, 7}; /* A B C D E F G */
    char c = (char)toupper((unsigned char)t[0]);
    if (c >= 'A' && c <= 'G') {
        int pc = base[c - 'A'];
        const char *p = t + 1;
        while (*p == '#' || *p == 'b') {
            pc += *p == '#' ? 1 : -1;
            p++;
        }
        char *end;
        long oct = strtol(p, &end, 10);
        if (end == p || *end) return -1000.0f;
        return (float)(12 * (oct + 1) + pc);
    }
    char *end;
    double v = strtod(t, &end);
    return (end == t || *end) ? -1000.0f : (float)v;
}

static int parse_send(Score *s, const char *v, Send *out)
{
    char name[32];
    const char *colon = strchr(v, ':');
    size_t n = colon ? (size_t)(colon - v) : strlen(v);
    if (n >= sizeof name) return -1;
    memcpy(name, v, n);
    name[n] = 0;
    out->aux = find_aux(s, name);
    out->gain = st_db_to_gain(colon ? (float)atof(colon + 1) : 0.0f);
    return out->aux < 0 ? -1 : 0;
}

static int parse_comp(const char *v, StCompParams *c)
{
    float t = -18, r = 2, a = 0.01f, rel = 0.2f, mk = 0;
    int n = sscanf(v, "%f:%f:%f:%f:%f", &t, &r, &a, &rel, &mk);
    if (n < 2) return -1;
    c->threshold_db = t;
    c->ratio = r;
    c->attack = a;
    c->release = rel;
    c->knee_db = 6.0f;
    c->makeup_db = mk;
    return 0;
}

static StWave parse_wave(const char *v)
{
    if (!strcmp(v, "square")) return ST_WAVE_SQUARE;
    if (!strcmp(v, "pulse")) return ST_WAVE_PULSE;
    if (!strcmp(v, "triangle")) return ST_WAVE_TRIANGLE;
    if (!strcmp(v, "sine")) return ST_WAVE_SINE;
    if (!strcmp(v, "noise")) return ST_WAVE_NOISE;
    return ST_WAVE_SAW;
}

static StFilterMode parse_filter(const char *v)
{
    if (!strcmp(v, "lp")) return ST_FILTER_LP;
    if (!strcmp(v, "hp")) return ST_FILTER_HP;
    if (!strcmp(v, "bp")) return ST_FILTER_BP;
    if (!strcmp(v, "ladder")) return ST_FILTER_LADDER;
    return ST_FILTER_NONE;
}

static StLfoShape parse_lfo_shape(const char *v)
{
    if (!strncmp(v, "tri", 3)) return ST_LFO_TRIANGLE;
    if (!strncmp(v, "square", 6)) return ST_LFO_SQUARE;
    if (!strncmp(v, "sh", 2) || !strncmp(v, "sample", 6)) return ST_LFO_SAMPLE_HOLD;
    return ST_LFO_SINE;
}

/* One key=value of a synth patch; 1 if the key was a synth key. */
static int synth_key(StSynthPatch *p, const char *k, const char *v)
{
    float f = (float)atof(v);
    if (!strcmp(k, "wave")) p->wave = parse_wave(v);
    else if (!strcmp(k, "pw")) p->pulse_width = f;
    else if (!strcmp(k, "stack")) p->stack = atoi(v);
    else if (!strcmp(k, "detune")) p->detune_cents = f;
    else if (!strcmp(k, "spread")) p->stereo_spread = f;
    else if (!strcmp(k, "sub")) p->sub_level = f;
    else if (!strcmp(k, "noise")) p->noise_level = f;
    else if (!strcmp(k, "attack")) p->amp.attack = f;
    else if (!strcmp(k, "decay")) p->amp.decay = f;
    else if (!strcmp(k, "sustain")) p->amp.sustain = f;
    else if (!strcmp(k, "release")) p->amp.release = f;
    else if (!strcmp(k, "filter")) p->filter = parse_filter(v);
    else if (!strcmp(k, "cutoff")) p->cutoff_hz = f;
    else if (!strcmp(k, "res")) p->resonance = f;
    else if (!strcmp(k, "keytrack")) p->key_track = f;
    else if (!strcmp(k, "fenv")) p->env_amount_oct = f;
    else if (!strcmp(k, "fattack")) p->filter_env.attack = f;
    else if (!strcmp(k, "fdecay")) p->filter_env.decay = f;
    else if (!strcmp(k, "fsustain")) p->filter_env.sustain = f;
    else if (!strcmp(k, "frelease")) p->filter_env.release = f;
    else if (!strcmp(k, "velcutoff")) p->velocity_to_cutoff_oct = f;
    else if (!strcmp(k, "drive")) p->drive = f;
    else if (!strcmp(k, "pitchenv")) p->pitch_env_semis = f;
    else if (!strcmp(k, "pitchtime")) p->pitch_env_time = f;
    else if (!strcmp(k, "glide")) p->glide_time = f;
    else if (!strcmp(k, "mono")) p->mono = atoi(v);
    else if (!strncmp(k, "lfo", 3) && (k[3] == '1' || k[3] == '2')) {
        StLfo *l = &p->lfo[k[3] - '1'];
        const char *sub = k + 4;
        if (!*sub) {
            char shape[16] = {0};
            float rate = l->rate_hz, delay = 0;
            sscanf(v, "%15[^:]:%f:%f", shape, &rate, &delay);
            l->shape = parse_lfo_shape(shape);
            l->rate_hz = rate;
            l->delay = delay;
        } else if (!strcmp(sub, "_pitch")) l->to_pitch_cents = f;
        else if (!strcmp(sub, "_cutoff")) l->to_cutoff_oct = f;
        else if (!strcmp(sub, "_amp")) l->to_amp = f;
        else if (!strcmp(sub, "_pan")) l->to_pan = f;
        else if (!strcmp(sub, "_pw")) l->to_pulse_width = f;
        else return 0;
    } else return 0;
    return 1;
}

static int sampler_key(StSamplerPatch *p, const char *k, const char *v)
{
    float f = (float)atof(v);
    if (!strcmp(k, "attack")) p->amp.attack = f;
    else if (!strcmp(k, "decay")) p->amp.decay = f;
    else if (!strcmp(k, "sustain")) p->amp.sustain = f;
    else if (!strcmp(k, "release")) p->amp.release = f;
    else if (!strcmp(k, "velcurve")) p->velocity_curve = f;
    else if (!strcmp(k, "offset")) p->start_offset = f;
    else if (!strcmp(k, "tune")) p->tune_cents = f;
    else if (!strcmp(k, "stack")) p->stack = atoi(v);
    else if (!strcmp(k, "stack_detune")) p->stack_detune_cents = f;
    else if (!strcmp(k, "stack_delay")) p->stack_delay = f;
    else if (!strcmp(k, "spread")) p->stereo_spread = f;
    else return 0;
    return 1;
}

static void dir_of(const char *path, char *out, size_t n)
{
    const char *slash = strrchr(path, '/');
    size_t len = slash ? (size_t)(slash - path) : 0;
    if (len >= n) len = n - 1;
    if (len) memcpy(out, path, len);
    out[len] = 0;
}

static void join(const char *dir, const char *rel, char *out, size_t n)
{
    if (rel[0] == '/' || !dir[0]) snprintf(out, n, "%s", rel);
    else snprintf(out, n, "%s/%s", dir, rel);
}

static int parse_file(Score *s, const char *path, int depth);

static int add_note(Score *s, Note n)
{
    if (s->nnotes == s->capnotes) {
        int cap = s->capnotes ? s->capnotes * 2 : 1024;
        Note *p = realloc(s->notes, (size_t)cap * sizeof *p);
        if (!p) return -1;
        s->notes = p;
        s->capnotes = cap;
    }
    s->notes[s->nnotes++] = n;
    return 0;
}

static int add_ramp(Score *s, Ramp r)
{
    if (s->nramps == s->capramps) {
        int cap = s->capramps ? s->capramps * 2 : 256;
        Ramp *p = realloc(s->ramps, (size_t)cap * sizeof *p);
        if (!p) return -1;
        s->ramps = p;
        s->capramps = cap;
    }
    s->ramps[s->nramps++] = r;
    return 0;
}

static int parse_line(Score *s, const char *file, int ln, const char *dir, char *line, int depth)
{
    char *tok[64];
    int nt = 0;
    /* `#` opens a comment at the start of a line or after a space; inside
     * a word it is a sharp (F#3). */
    for (char *h = line; *h; h++) {
        if (*h == '#' && (h == line || isspace((unsigned char)h[-1]))) {
            *h = 0;
            break;
        }
    }
    for (char *t = strtok(line, " \t\r\n"); t && nt < 64; t = strtok(NULL, " \t\r\n"))
        tok[nt++] = t;
    if (!nt) return 0;
    const char *cmd = tok[0];

    if (!strcmp(cmd, "score") || !strcmp(cmd, "title")) return 0;
    if (!strcmp(cmd, "rate") && nt > 1) { s->rate = atoi(tok[1]); return 0; }
    if (!strcmp(cmd, "length") && nt > 1) { s->length = atof(tok[1]); return 0; }
    if (!strcmp(cmd, "loop") && nt > 1) { s->loop = !strcmp(tok[1], "on"); return 0; }
    if (!strcmp(cmd, "tail") && nt > 1) { s->tail = atof(tok[1]); return 0; }
    if (!strcmp(cmd, "seed") && nt > 1) { s->seed = (uint32_t)strtoul(tok[1], NULL, 10); return 0; }
    if (!strcmp(cmd, "tempo") && nt > 2) {
        if (s->ntempo == MAX_TEMPO) return fail(s, file, ln, "too many tempo changes", NULL);
        s->tempo_beat[s->ntempo] = atof(tok[1]);
        s->tempo_bpm[s->ntempo] = atof(tok[2]);
        if (s->tempo_bpm[s->ntempo] <= 0) return fail(s, file, ln, "bad tempo", tok[2]);
        s->ntempo++;
        return 0;
    }
    if (!strcmp(cmd, "include") && nt > 1) {
        char p[1024];
        if (depth >= MAX_DEPTH) return fail(s, file, ln, "include nested too deep", tok[1]);
        join(dir, tok[1], p, sizeof p);
        return parse_file(s, p, depth + 1);
    }

    if (!strcmp(cmd, "aux") && nt > 2) {
        if (s->naux == MAX_AUX) return fail(s, file, ln, "too many aux returns", NULL);
        Aux *a = &s->aux[s->naux];
        memset(a, 0, sizeof *a);
        snprintf(a->name, sizeof a->name, "%s", tok[1]);
        if (!strcmp(tok[2], "reverb")) {
            a->kind = 0;
            st_reverb_preset("hall", &a->rp);
        } else if (!strcmp(tok[2], "delay")) {
            a->kind = 1;
            a->time_beats_l = a->time_beats_r = 0.75;
            a->dp.feedback = 0.3f;
            a->dp.low_cut_hz = 150;
            a->dp.high_cut_hz = 9000;
        } else return fail(s, file, ln, "unknown aux kind", tok[2]);
        for (int i = 3; i < nt; i++) {
            char *eq = strchr(tok[i], '=');
            if (!eq) return fail(s, file, ln, "expected key=value", tok[i]);
            *eq = 0;
            const char *k = tok[i], *v = eq + 1;
            float f = (float)atof(v);
            if (!strcmp(k, "gain")) a->gain_db = f;
            else if (a->kind == 0 && !strcmp(k, "preset")) {
                if (!st_reverb_preset(v, &a->rp)) return fail(s, file, ln, "unknown reverb preset", v);
            } else if (a->kind == 0 && !strcmp(k, "rt60")) a->rp.rt60 = f;
            else if (a->kind == 0 && !strcmp(k, "predelay")) a->rp.predelay = f;
            else if (a->kind == 0 && !strcmp(k, "size")) a->rp.size = f;
            else if (a->kind == 0 && !strcmp(k, "damping")) a->rp.damping_hz = f;
            else if (a->kind == 0 && !strcmp(k, "lowcut")) a->rp.low_cut_hz = f;
            else if (a->kind == 0 && !strcmp(k, "width")) a->rp.width = f;
            else if (a->kind == 0 && !strcmp(k, "mod")) a->rp.mod_depth = f;
            else if (a->kind == 1 && !strcmp(k, "time")) a->time_beats_l = a->time_beats_r = atof(v);
            else if (a->kind == 1 && !strcmp(k, "time_r")) a->time_beats_r = atof(v);
            else if (a->kind == 1 && !strcmp(k, "feedback")) a->dp.feedback = f;
            else if (a->kind == 1 && !strcmp(k, "pingpong")) a->dp.ping_pong = atoi(v);
            else if (a->kind == 1 && !strcmp(k, "lowcut")) a->dp.low_cut_hz = f;
            else if (a->kind == 1 && !strcmp(k, "highcut")) a->dp.high_cut_hz = f;
            else return fail(s, file, ln, "unknown aux key", k);
        }
        s->naux++;
        return 0;
    }

    if (!strcmp(cmd, "bus") || !strcmp(cmd, "master")) {
        int master = !strcmp(cmd, "master");
        Bus *b = NULL;
        int first = 1;
        if (!master) {
            if (nt < 2) return fail(s, file, ln, "bus needs a name", NULL);
            if (s->nbus == MAX_BUS) return fail(s, file, ln, "too many buses", NULL);
            b = &s->bus[s->nbus];
            memset(b, 0, sizeof *b);
            b->key = -1;
            snprintf(b->name, sizeof b->name, "%s", tok[1]);
            first = 2;
        }
        for (int i = first; i < nt; i++) {
            char *eq = strchr(tok[i], '=');
            if (!eq) return fail(s, file, ln, "expected key=value", tok[i]);
            *eq = 0;
            const char *k = tok[i], *v = eq + 1;
            if (master) {
                if (!strcmp(k, "gain")) s->master_gain_db = (float)atof(v);
                else if (!strcmp(k, "comp")) {
                    if (parse_comp(v, &s->master_compp)) return fail(s, file, ln, "bad comp", v);
                    s->master_has_comp = 1;
                } else if (!strcmp(k, "limit")) { s->limit_db = (float)atof(v); s->has_limit = 1; }
                else return fail(s, file, ln, "unknown master key", k);
                continue;
            }
            float a1 = 0, a2 = 0, a3 = 0;
            if (!strcmp(k, "gain")) b->gain_db = (float)atof(v);
            else if (!strcmp(k, "hpf") || !strcmp(k, "lpf") || !strcmp(k, "lowshelf") ||
                     !strcmp(k, "highshelf") || !strcmp(k, "peak")) {
                if (b->neq == MAX_EQ) return fail(s, file, ln, "too many EQ bands", k);
                EqSpec *q = &b->eqspec[b->neq++];
                int n = sscanf(v, "%f:%f:%f", &a1, &a2, &a3);
                q->hz = a1;
                q->a = a2;
                q->b = a3;
                q->type = !strcmp(k, "hpf") ? 0 : !strcmp(k, "lpf") ? 1 : !strcmp(k, "lowshelf") ? 2
                        : !strcmp(k, "highshelf") ? 3 : 4;
                if (q->type == 4 && n != 3) return fail(s, file, ln, "peak needs Hz:q:dB", v);
            } else if (!strcmp(k, "comp")) {
                if (parse_comp(v, &b->compp)) return fail(s, file, ln, "bad comp", v);
                b->has_comp = 1;
            } else if (!strcmp(k, "key")) {
                b->key = find_inst(s, v);
                if (b->key < 0) return fail(s, file, ln, "key: unknown instrument (declare it first)", v);
            } else if (!strcmp(k, "send")) {
                if (b->nsend == MAX_SENDS || parse_send(s, v, &b->sends[b->nsend]))
                    return fail(s, file, ln, "bad send (declare the aux first)", v);
                b->nsend++;
            } else return fail(s, file, ln, "unknown bus key", k);
        }
        if (!master) s->nbus++;
        return 0;
    }

    if (!strcmp(cmd, "inst") && nt > 2) {
        if (s->ninst == MAX_INST) return fail(s, file, ln, "too many instruments", NULL);
        if (find_inst(s, tok[1]) >= 0) return fail(s, file, ln, "instrument declared twice", tok[1]);
        Inst *in = &s->inst[s->ninst];
        memset(in, 0, sizeof *in);
        snprintf(in->name, sizeof in->name, "%s", tok[1]);
        in->bus = -1;
        in->poly = DEFAULT_POLY;
        st_synth_patch_default(&in->synth);
        st_sampler_patch_default(&in->samp);
        for (int i = 2; i < nt; i++) {
            if (!strcmp(tok[i], "synth")) { in->kind = KIND_SYNTH; continue; }
            char *eq = strchr(tok[i], '=');
            if (!eq) return fail(s, file, ln, "expected key=value", tok[i]);
            *eq = 0;
            const char *k = tok[i], *v = eq + 1;
            if (!strcmp(k, "sfz")) {
                in->kind = KIND_SAMPLER;
                join(dir, v, in->sfz, sizeof in->sfz);
            } else if (!strcmp(k, "bus")) {
                in->bus = find_bus(s, v);
                if (in->bus < 0) return fail(s, file, ln, "unknown bus (declare it first)", v);
            } else if (!strcmp(k, "gain")) in->gain_db = (float)atof(v);
            else if (!strcmp(k, "pan")) in->pan = (float)atof(v);
            else if (!strcmp(k, "poly")) in->poly = atoi(v);
            else if (!strcmp(k, "send")) {
                if (in->nsend == MAX_SENDS || parse_send(s, v, &in->sends[in->nsend]))
                    return fail(s, file, ln, "bad send (declare the aux first)", v);
                in->nsend++;
            } else if (in->kind == KIND_SAMPLER ? !sampler_key(&in->samp, k, v)
                                                : !synth_key(&in->synth, k, v))
                return fail(s, file, ln, "unknown instrument key", k);
        }
        s->ninst++;
        return 0;
    }

    if (!strcmp(cmd, "note")) {
        if (nt < 6) return fail(s, file, ln, "note <beat> <beats> <inst> <pitch> <velocity>", NULL);
        Note n = {0};
        n.beat = atof(tok[1]);
        n.dur = atof(tok[2]);
        n.inst = find_inst(s, tok[3]);
        n.pitch = parse_pitch(tok[4]);
        n.vel = atoi(tok[5]);
        n.legato = nt > 6 && !strcmp(tok[6], "legato");
        if (n.inst < 0) return fail(s, file, ln, "unknown instrument", tok[3]);
        if (n.pitch < 0) return fail(s, file, ln, "bad pitch", tok[4]);
        if (n.vel < 1 || n.vel > 127 || n.dur <= 0) return fail(s, file, ln, "bad velocity or length", NULL);
        return add_note(s, n);
    }

    if (!strcmp(cmd, "ramp")) {
        if (nt < 6) return fail(s, file, ln, "ramp <target>.<param> <beat0> <beat1> <from> <to>", NULL);
        Ramp r = {0};
        char *dot = strrchr(tok[1], '.');
        if (!dot) return fail(s, file, ln, "ramp target needs .param", tok[1]);
        *dot = 0;
        const char *param = dot + 1;
        if (!strcmp(tok[1], "master")) { r.tkind = T_MASTER; r.index = 0; }
        else if ((r.index = find_inst(s, tok[1])) >= 0) r.tkind = T_INST;
        else if ((r.index = find_bus(s, tok[1])) >= 0) r.tkind = T_BUS;
        else if ((r.index = find_aux(s, tok[1])) >= 0) r.tkind = T_AUX;
        else return fail(s, file, ln, "unknown ramp target", tok[1]);
        if (!strcmp(param, "gain")) r.param = P_GAIN;
        else if (!strcmp(param, "cutoff") && r.tkind == T_INST) r.param = P_CUTOFF;
        else if (!strcmp(param, "pitch") && r.tkind == T_INST) r.param = P_PITCH;
        else return fail(s, file, ln, "unknown ramp parameter", param);
        r.b0 = atof(tok[2]);
        r.b1 = atof(tok[3]);
        r.v0 = (float)atof(tok[4]);
        r.v1 = (float)atof(tok[5]);
        r.smooth = nt > 6 && !strcmp(tok[6], "smooth");
        if (r.b1 < r.b0) return fail(s, file, ln, "ramp ends before it starts", NULL);
        return add_ramp(s, r);
    }
    return fail(s, file, ln, "unknown line", cmd);
}

static int parse_file(Score *s, const char *path, int depth)
{
    FILE *f = fopen(path, "r");
    if (!f) return fail(s, path, 0, "cannot open", NULL);
    char dir[512];
    dir_of(path, dir, sizeof dir);
    char line[4096];
    int ln = 0, rc = 0;
    while (!rc && fgets(line, sizeof line, f)) {
        ln++;
        rc = parse_line(s, path, ln, dir, line, depth);
    }
    fclose(f);
    return rc;
}

/* ------------------------------------------------------------------ */
/* Time                                                                 */

static int cmp_tempo(const Score *s, int a, int b) { return s->tempo_beat[a] > s->tempo_beat[b]; }

static void sort_tempo(Score *s)
{
    for (int i = 1; i < s->ntempo; i++)
        for (int j = i; j > 0 && cmp_tempo(s, j - 1, j); j--) {
            double tb = s->tempo_beat[j], tp = s->tempo_bpm[j];
            s->tempo_beat[j] = s->tempo_beat[j - 1];
            s->tempo_bpm[j] = s->tempo_bpm[j - 1];
            s->tempo_beat[j - 1] = tb;
            s->tempo_bpm[j - 1] = tp;
        }
}

/* Seconds from beat 0 to `beat` through the tempo map. */
static double beat_secs(const Score *s, double beat)
{
    double t = 0, from = 0, bpm = s->ntempo ? s->tempo_bpm[0] : 120.0;
    for (int i = 0; i < s->ntempo; i++) {
        double b = s->tempo_beat[i];
        if (b <= from) { bpm = s->tempo_bpm[i]; continue; }
        if (beat <= b) break;
        t += (b - from) * 60.0 / bpm;
        from = b;
        bpm = s->tempo_bpm[i];
    }
    return t + (beat - from) * 60.0 / bpm;
}

/* Value of a lane at `beat`: the last ramp started at or before it. */
static float lane(const Score *s, int tkind, int index, int param, double beat, float dflt)
{
    const Ramp *cur = NULL;
    for (int i = 0; i < s->nramps; i++) {
        const Ramp *r = &s->ramps[i];
        if (r->tkind != tkind || r->index != index || r->param != param || r->b0 > beat) continue;
        if (!cur || r->b0 >= cur->b0) cur = r;
    }
    if (!cur) return dflt;
    if (beat >= cur->b1 || cur->b1 <= cur->b0) return cur->v1;
    double x = (beat - cur->b0) / (cur->b1 - cur->b0);
    if (cur->smooth) x = x * x * (3 - 2 * x);
    return (float)(cur->v0 + (cur->v1 - cur->v0) * x);
}

/* ------------------------------------------------------------------ */
/* Render                                                               */

static int cmp_event(const void *a, const void *b)
{
    const Event *x = a, *y = b;
    if (x->frame != y->frame) return x->frame < y->frame ? -1 : 1;
    if (x->on != y->on) return x->on - y->on;  /* offs before ons */
    if (x->pass != y->pass) return x->pass - y->pass;
    return x->note - y->note;
}

static uint32_t mix_seed(uint32_t a, uint32_t b)
{
    uint32_t h = a * 0x9E3779B1u ^ (b + 0x7F4A7C15u + (a << 6) + (a >> 2));
    return h ? h : 1u;
}

static void pan_gains(float pan, float *gl, float *gr)
{
    if (pan < -1) pan = -1;
    if (pan > 1) pan = 1;
    float th = (pan + 1.0f) * 0.78539816f; /* 0..pi/2 */
    *gl = cosf(th) * 1.41421356f;
    *gr = sinf(th) * 1.41421356f;
}

static void free_score(Score *s)
{
    for (int i = 0; i < s->ninst; i++) {
        /* sets are shared between instruments naming the same map */
        StSampleSet *set = s->inst[i].set;
        if (!set) continue;
        for (int j = i; j < s->ninst; j++)
            if (s->inst[j].set == set) s->inst[j].set = NULL;
        st_sampleset_free(set);
    }
    for (int i = 0; i < s->naux; i++) {
        if (!s->aux[i].ready) continue;
        if (s->aux[i].kind == 0) st_reverb_free(&s->aux[i].rv);
        else st_delay_free(&s->aux[i].dl);
    }
    free(s->notes);
    free(s->ramps);
}

int st_render_score(const char *path, int rate, StRender *out, char *err, size_t errlen)
{
    static Score s; /* large: keep it off the stack */
    static Voice voices[MAX_VOICES];
    static float ibuf[MAX_INST][2][ST_BLOCK];
    static float bbuf[MAX_BUS][2][ST_BLOCK];
    static float abuf_in[MAX_AUX][2][ST_BLOCK], abuf_out[MAX_AUX][2][ST_BLOCK];
    static float mbuf[2][ST_BLOCK];
    StLimiter lim;
    int have_lim = 0, rc = -1;
    Event *ev = NULL;
    float *pcm = NULL;

    memset(&s, 0, sizeof s);
    memset(voices, 0, sizeof voices);
    memset(out, 0, sizeof *out);
    s.err = err;
    s.errlen = errlen;
    s.rate = rate > 0 ? rate : ST_DEFAULT_RATE;
    s.seed = 1;
    if (parse_file(&s, path, 0)) goto done;
    if (rate > 0) s.rate = rate;
    if (!s.ntempo) { s.tempo_beat[0] = 0; s.tempo_bpm[0] = 120; s.ntempo = 1; }
    sort_tempo(&s);
    if (s.length <= 0) { fail(&s, path, 0, "missing length", NULL); goto done; }
    const float R = (float)s.rate;

    /* Instruments: default bus, sample maps (shared by path). */
    if (!s.nbus) {
        memset(&s.bus[0], 0, sizeof s.bus[0]);
        snprintf(s.bus[0].name, sizeof s.bus[0].name, "main");
        s.bus[0].key = -1;
        s.nbus = 1;
    }
    for (int i = 0; i < s.ninst; i++) {
        Inst *in = &s.inst[i];
        if (in->bus < 0) in->bus = 0;
        if (in->poly < 1) in->poly = 1;
        if (in->kind != KIND_SAMPLER) continue;
        for (int j = 0; j < i; j++)
            if (s.inst[j].kind == KIND_SAMPLER && !strcmp(s.inst[j].sfz, in->sfz)) in->set = s.inst[j].set;
        if (!in->set) {
            char e[256];
            in->set = st_sfz_load(in->sfz, e, sizeof e);
            if (!in->set) { fail(&s, path, 0, "cannot load sample map:", e); goto done; }
        }
    }
    /* Buses (EQ was set at parse time at the score's rate) and effects. */
    for (int b = 0; b < s.nbus; b++) {
        Bus *bs = &s.bus[b];
        for (int q = 0; q < bs->neq; q++) {
            const EqSpec *e = &bs->eqspec[q];
            StBiquad *bq = &bs->eq[q];
            if (e->type == 0) st_biquad_highpass(bq, R, e->hz, 0.7071f);
            else if (e->type == 1) st_biquad_lowpass(bq, R, e->hz, 0.7071f);
            else if (e->type == 2) st_biquad_lowshelf(bq, R, e->hz, e->a);
            else if (e->type == 3) st_biquad_highshelf(bq, R, e->hz, e->a);
            else st_biquad_peak(bq, R, e->hz, e->a, e->b);
        }
        if (bs->has_comp) st_comp_init(&bs->comp, R, &bs->compp);
    }
    if (s.master_has_comp) st_comp_init(&s.master_comp, R, &s.master_compp);
    double spb0 = 60.0 / s.tempo_bpm[0];
    for (int a = 0; a < s.naux; a++) {
        Aux *x = &s.aux[a];
        if (x->kind == 0) {
            if (st_reverb_init(&x->rv, R, &x->rp)) { fail(&s, path, 0, "reverb init failed", x->name); goto done; }
        } else {
            x->dp.time_l = (float)(x->time_beats_l * spb0);
            x->dp.time_r = (float)(x->time_beats_r * spb0);
            float mx = (x->dp.time_l > x->dp.time_r ? x->dp.time_l : x->dp.time_r) * 2.0f + 0.1f;
            if (st_delay_init(&x->dl, R, &x->dp, mx)) { fail(&s, path, 0, "delay init failed", x->name); goto done; }
        }
        x->ready = 1;
    }
    if (s.has_limit) {
        if (st_limiter_init(&lim, R, s.limit_db, 0.005f, 0.08f)) { fail(&s, path, 0, "limiter init failed", NULL); goto done; }
        have_lim = 1;
    }
    int latency = have_lim ? st_limiter_latency(&lim) : 0;

    /* The span to render: two passes for a loop (the second is kept), or
     * the piece and its tail, plus the limiter's latency. */
    double len_secs = beat_secs(&s, s.length);
    long pass_frames = (long)llround(len_secs * R);
    int passes = s.loop ? 2 : 1;
    long keep_from = (s.loop ? pass_frames : 0) + latency;
    long keep_frames = s.loop ? pass_frames : pass_frames + (long)llround(s.tail * R);
    long total = keep_from + keep_frames;

    /* Events (note-on and note-off) for every pass, sorted. */
    int nev = 0;
    ev = malloc(sizeof *ev * (size_t)(s.nnotes * 2 * passes + 1));
    pcm = calloc((size_t)keep_frames * 2, sizeof *pcm);
    if (!ev || !pcm) { fail(&s, path, 0, "out of memory", NULL); goto done; }
    for (int p = 0; p < passes; p++)
        for (int i = 0; i < s.nnotes; i++) {
            const Note *n = &s.notes[i];
            long on = (long)llround((beat_secs(&s, n->beat) ) * R) + p * pass_frames;
            long off = (long)llround(beat_secs(&s, n->beat + n->dur) * R) + p * pass_frames;
            if (off <= on) off = on + 1;
            ev[nev++] = (Event){on, 1, i, p};
            ev[nev++] = (Event){off, 0, i, p};
        }
    qsort(ev, (size_t)nev, sizeof *ev, cmp_event);

    int ei = 0;
    long pos = 0;
    while (pos < total) {
        /* Events due now. */
        while (ei < nev && ev[ei].frame <= pos) {
            const Event *e = &ev[ei++];
            const Note *n = &s.notes[e->note];
            Inst *in = &s.inst[n->inst];
            if (!e->on) {
                for (int v = 0; v < MAX_VOICES; v++) {
                    Voice *vc = &voices[v];
                    if (!vc->active || vc->released || vc->note != e->note || vc->pass != e->pass) continue;
                    vc->released = 1;
                    if (in->kind == KIND_SYNTH) st_synth_release(&vc->syn);
                    else st_sampler_release(&vc->smp);
                }
                continue;
            }
            /* A mono synth glides into a held note instead of starting one. */
            if (in->kind == KIND_SYNTH && in->synth.mono) {
                Voice *held = NULL;
                for (int v = 0; v < MAX_VOICES; v++)
                    if (voices[v].active && voices[v].inst == n->inst && !voices[v].released) held = &voices[v];
                if (held) {
                    st_synth_legato(&held->syn, n->pitch, n->vel / 127.0f);
                    held->note = e->note;
                    held->pass = e->pass;
                    continue;
                }
            }
            /* Polyphony: steal this instrument's oldest voice past its limit
             * (released ones first), else a free slot, else the oldest. */
            int count = 0, oldest = -1, oldest_rel = -1, slot = -1;
            for (int v = 0; v < MAX_VOICES; v++) {
                Voice *vc = &voices[v];
                if (!vc->active) { if (slot < 0) slot = v; continue; }
                if (vc->inst != n->inst) continue;
                count++;
                if (oldest < 0 || vc->started < voices[oldest].started) oldest = v;
                if (vc->released && (oldest_rel < 0 || vc->started < voices[oldest_rel].started)) oldest_rel = v;
            }
            if (count >= in->poly) slot = oldest_rel >= 0 ? oldest_rel : oldest;
            if (slot < 0) {
                for (int v = 0; v < MAX_VOICES; v++)
                    if (slot < 0 || voices[v].started < voices[slot].started) slot = v;
            }
            Voice *vc = &voices[slot];
            memset(vc, 0, sizeof *vc);
            vc->inst = n->inst;
            vc->note = e->note;
            vc->pass = e->pass;
            vc->started = pos;
            uint32_t seed = mix_seed(s.seed, (uint32_t)(e->note * 2 + e->pass));
            if (in->kind == KIND_SYNTH) {
                st_synth_start(&vc->syn, &in->synth, R, n->pitch, n->vel / 127.0f, seed);
                vc->active = 1;
            } else {
                vc->active = st_sampler_start(&vc->smp, in->set, &in->samp, R, n->pitch, n->vel, in->rr++, seed);
            }
        }

        long next = ei < nev ? ev[ei].frame : total;
        int n = (int)((next - pos) < ST_BLOCK ? (next - pos) : ST_BLOCK);
        if (n <= 0) n = 1;
        if (pos + n > total) n = (int)(total - pos);

        /* Beats at the block's start and end (passes repeat the score). */
        double t0 = (double)pos / R, t1 = (double)(pos + n) / R;
        double pass0 = s.loop && t0 >= len_secs ? len_secs : 0.0;
        /* seconds -> beats by inverting the tempo map numerically (bisection) */
        double b0, b1;
        {
            double lo = 0, hi = s.length * 2 + 1e6;
            double target = t0 - pass0;
            for (int it = 0; it < 60; it++) { double mid = (lo + hi) / 2; if (beat_secs(&s, mid) < target) lo = mid; else hi = mid; }
            b0 = lo;
            lo = 0; hi = s.length * 2 + 1e6; target = t1 - pass0;
            for (int it = 0; it < 60; it++) { double mid = (lo + hi) / 2; if (beat_secs(&s, mid) < target) lo = mid; else hi = mid; }
            b1 = lo;
        }

        memset(ibuf, 0, sizeof ibuf);
        memset(bbuf, 0, sizeof bbuf);
        memset(abuf_in, 0, sizeof abuf_in);
        memset(abuf_out, 0, sizeof abuf_out);
        memset(mbuf, 0, sizeof mbuf);

        /* Voices into their instrument. */
        for (int v = 0; v < MAX_VOICES; v++) {
            Voice *vc = &voices[v];
            if (!vc->active) continue;
            StVoiceMod mod = {1.0f, 1.0f,
                              lane(&s, T_INST, vc->inst, P_CUTOFF, b0, 0.0f),
                              lane(&s, T_INST, vc->inst, P_PITCH, b0, 0.0f)};
            int alive = s.inst[vc->inst].kind == KIND_SYNTH
                ? st_synth_render(&vc->syn, &mod, ibuf[vc->inst][0], ibuf[vc->inst][1], n)
                : st_sampler_render(&vc->smp, &mod, ibuf[vc->inst][0], ibuf[vc->inst][1], n);
            if (!alive) vc->active = 0;
        }
        /* Instruments: gain (with its lane), pan, into the bus and sends. */
        for (int i = 0; i < s.ninst; i++) {
            Inst *in = &s.inst[i];
            float g0 = st_db_to_gain(in->gain_db + lane(&s, T_INST, i, P_GAIN, b0, 0.0f));
            float g1 = st_db_to_gain(in->gain_db + lane(&s, T_INST, i, P_GAIN, b1, 0.0f));
            float pl, pr;
            pan_gains(in->pan, &pl, &pr);
            for (int k = 0; k < n; k++) {
                float g = g0 + (g1 - g0) * (float)k / (float)n;
                float l = ibuf[i][0][k] * g * pl, r = ibuf[i][1][k] * g * pr;
                ibuf[i][0][k] = l;
                ibuf[i][1][k] = r;
                bbuf[in->bus][0][k] += l;
                bbuf[in->bus][1][k] += r;
                for (int sd = 0; sd < in->nsend; sd++) {
                    abuf_in[in->sends[sd].aux][0][k] += l * in->sends[sd].gain;
                    abuf_in[in->sends[sd].aux][1][k] += r * in->sends[sd].gain;
                }
            }
        }
        /* Buses: EQ, compressor (optionally keyed), gain, sends, master. */
        for (int b = 0; b < s.nbus; b++) {
            Bus *bs = &s.bus[b];
            for (int q = 0; q < bs->neq; q++) st_biquad_process(&bs->eq[q], bbuf[b][0], bbuf[b][1], n);
            if (bs->has_comp)
                st_comp_process(&bs->comp, bbuf[b][0], bbuf[b][1],
                                bs->key >= 0 ? ibuf[bs->key][0] : NULL,
                                bs->key >= 0 ? ibuf[bs->key][1] : NULL, n);
            float g0 = st_db_to_gain(bs->gain_db + lane(&s, T_BUS, b, P_GAIN, b0, 0.0f));
            float g1 = st_db_to_gain(bs->gain_db + lane(&s, T_BUS, b, P_GAIN, b1, 0.0f));
            for (int k = 0; k < n; k++) {
                float g = g0 + (g1 - g0) * (float)k / (float)n;
                float l = bbuf[b][0][k] * g, r = bbuf[b][1][k] * g;
                mbuf[0][k] += l;
                mbuf[1][k] += r;
                for (int sd = 0; sd < bs->nsend; sd++) {
                    abuf_in[bs->sends[sd].aux][0][k] += l * bs->sends[sd].gain;
                    abuf_in[bs->sends[sd].aux][1][k] += r * bs->sends[sd].gain;
                }
            }
        }
        /* Aux returns into the master. */
        for (int a = 0; a < s.naux; a++) {
            Aux *x = &s.aux[a];
            if (x->kind == 0) st_reverb_process(&x->rv, abuf_in[a][0], abuf_in[a][1], abuf_out[a][0], abuf_out[a][1], n);
            else st_delay_process(&x->dl, abuf_in[a][0], abuf_in[a][1], abuf_out[a][0], abuf_out[a][1], n);
            float g0 = st_db_to_gain(x->gain_db + lane(&s, T_AUX, a, P_GAIN, b0, 0.0f));
            float g1 = st_db_to_gain(x->gain_db + lane(&s, T_AUX, a, P_GAIN, b1, 0.0f));
            for (int k = 0; k < n; k++) {
                float g = g0 + (g1 - g0) * (float)k / (float)n;
                mbuf[0][k] += abuf_out[a][0][k] * g;
                mbuf[1][k] += abuf_out[a][1][k] * g;
            }
        }
        /* Master. */
        {
            float g0 = st_db_to_gain(s.master_gain_db + lane(&s, T_MASTER, 0, P_GAIN, b0, 0.0f));
            float g1 = st_db_to_gain(s.master_gain_db + lane(&s, T_MASTER, 0, P_GAIN, b1, 0.0f));
            for (int k = 0; k < n; k++) {
                float g = g0 + (g1 - g0) * (float)k / (float)n;
                mbuf[0][k] *= g;
                mbuf[1][k] *= g;
            }
            if (s.master_has_comp) st_comp_process(&s.master_comp, mbuf[0], mbuf[1], NULL, NULL, n);
            if (have_lim) st_limiter_process(&lim, mbuf[0], mbuf[1], n);
        }
        for (int k = 0; k < n; k++) {
            long f = pos + k - keep_from;
            if (f < 0 || f >= keep_frames) continue;
            pcm[f * 2] = mbuf[0][k];
            pcm[f * 2 + 1] = mbuf[1][k];
        }
        pos += n;
    }

    out->frames = pcm;
    out->frame_count = (int)keep_frames;
    out->rate = s.rate;
    pcm = NULL;
    rc = 0;
done:
    if (have_lim) st_limiter_free(&lim);
    free(ev);
    free(pcm);
    free_score(&s);
    return rc;
}

void st_render_free(StRender *r)
{
    free(r->frames);
    r->frames = NULL;
    r->frame_count = 0;
}
