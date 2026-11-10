/*
 * studio_sampler.c: the studio's SFZ-subset sampler (BRYX addition; see
 * studio.h and studio_sampler.h).
 *
 * Loading (allocation, file I/O, FLAC decoding) happens in
 * st_sampleset_add_region / st_sfz_load. Playing (st_sampler_*) touches
 * only the voice struct and the set's read-only audio.
 */
#include <ctype.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define DR_FLAC_IMPLEMENTATION
#include "../vendor/dr_flac.h"

#include "studio_sampler.h"

/* ================================================================== */
/* The sample set                                                      */

struct StSampleSet {
    StRegion *regions;
    int nregions, region_cap;
    float **buffers; /* every audio buffer the regions point into */
    int nbuffers, buffer_cap;
};

StSampleSet *st_sampleset_new(void)
{
    return calloc(1, sizeof(StSampleSet));
}

void st_sampleset_free(StSampleSet *s)
{
    int i;
    if (!s)
        return;
    for (i = 0; i < s->nbuffers; i++)
        free(s->buffers[i]);
    free(s->buffers);
    free(s->regions);
    free(s);
}

int st_sampleset_region_count(const StSampleSet *s)
{
    return s ? s->nregions : 0;
}

const StRegion *st_sampleset_region(const StSampleSet *s, int i)
{
    if (!s || i < 0 || i >= s->nregions)
        return NULL;
    return &s->regions[i];
}

/* Takes ownership of `buf` (malloc'd). Returns 0 on success. */
static int set_adopt_buffer(StSampleSet *s, float *buf)
{
    if (s->nbuffers == s->buffer_cap) {
        int cap = s->buffer_cap ? s->buffer_cap * 2 : 16;
        float **nb = realloc(s->buffers, (size_t)cap * sizeof(float *));
        if (!nb)
            return -1;
        s->buffers = nb;
        s->buffer_cap = cap;
    }
    s->buffers[s->nbuffers++] = buf;
    return 0;
}

/* Why a region is unplayable, or NULL when it is fine. The loop needs a
 * few frames so a 4-tap read straddling the seam wraps at most once. */
static const char *region_problem(const StRegion *r)
{
    if (!r->data || r->frames <= 0 || r->sample_rate <= 0)
        return "no audio";
    if (r->lokey < 0 || r->hikey > 127 || r->lokey > r->hikey)
        return "bad key range (lokey/hikey)";
    if (r->lovel < 0 || r->hivel > 127 || r->lovel > r->hivel)
        return "bad velocity range (lovel/hivel)";
    if (r->keycenter < 0 || r->keycenter > 127)
        return "bad pitch_keycenter";
    if (r->seq_length < 1 || r->seq_position < 1 ||
        r->seq_position > r->seq_length)
        return "bad seq_length/seq_position";
    if (r->offset < 0 || r->offset >= r->frames)
        return "offset outside the sample";
    if (r->loop) {
        if (r->loop_start < 0 || r->loop_end >= r->frames)
            return "loop outside the sample";
        if (r->loop_end - r->loop_start + 1 < 4)
            return "loop shorter than 4 frames";
    }
    return NULL;
}

/* Adds a region pointing at audio the set already owns. */
static int set_push_region(StSampleSet *s, const StRegion *r)
{
    if (s->nregions == s->region_cap) {
        int cap = s->region_cap ? s->region_cap * 2 : 16;
        StRegion *nr = realloc(s->regions, (size_t)cap * sizeof(StRegion));
        if (!nr)
            return -1;
        s->regions = nr;
        s->region_cap = cap;
    }
    s->regions[s->nregions] = *r;
    return s->nregions++;
}

int st_sampleset_add_region(StSampleSet *s, const StRegion *r_template,
                            const float *data, int frames, int sample_rate)
{
    StRegion r;
    float *copy;
    if (!s || !r_template || !data || frames <= 0 || sample_rate <= 0)
        return -1;
    r = *r_template;
    r.data = data; /* checked against the caller's data, then swapped */
    r.frames = frames;
    r.sample_rate = sample_rate;
    if (region_problem(&r))
        return -1;
    copy = malloc((size_t)frames * sizeof(float));
    if (!copy)
        return -1;
    memcpy(copy, data, (size_t)frames * sizeof(float));
    if (set_adopt_buffer(s, copy) != 0) {
        free(copy);
        return -1;
    }
    r.data = copy;
    return set_push_region(s, &r);
}

/* ================================================================== */
/* The SFZ subset                                                      */

#define SFZ_PATH_MAX 1024

/* Which opcodes a scope (a header's block) has set. */
enum {
    OP_LOKEY = 1 << 0, OP_HIKEY = 1 << 1, OP_KEYCENTER = 1 << 2,
    OP_LOVEL = 1 << 3, OP_HIVEL = 1 << 4, OP_TUNE = 1 << 5,
    OP_VOLUME = 1 << 6, OP_LOOP = 1 << 7, OP_LOOP_START = 1 << 8,
    OP_LOOP_END = 1 << 9, OP_OFFSET = 1 << 10, OP_SEQ_LENGTH = 1 << 11,
    OP_SEQ_POSITION = 1 << 12, OP_SAMPLE = 1 << 13
};

typedef struct {
    unsigned set;
    StRegion r;               /* only the fields named in `set` count */
    char sample[SFZ_PATH_MAX];
} SfzScope;

/* Scope levels, outermost first; a header resets its level and every
 * level inside it. */
enum { LV_GLOBAL = 0, LV_MASTER, LV_GROUP, LV_REGION, LV_COUNT };

typedef struct {
    const char *path;         /* the .sfz, for messages */
    char dir[SFZ_PATH_MAX];   /* its directory, with a trailing '/' or empty */
    char default_path[SFZ_PATH_MAX];
    SfzScope scope[LV_COUNT];
    int level;                /* the header in force: LV_*, -1 for <control>,
                                 -2 for an unknown header (opcodes ignored) */
    int in_region;            /* a <region> is open and not yet flushed */
    int region_line;
    /* decoded files, so each is read once */
    char (*cache_path)[SFZ_PATH_MAX];
    int *cache_buffer;        /* index into the set's buffers */
    int *cache_frames, *cache_rate;
    int ncache, cache_cap;
    StSampleSet *set;
    char *err;
    size_t errlen;
} SfzParser;

static int sfz_fail(SfzParser *p, int line, const char *fmt, const char *arg)
{
    if (p->err && p->errlen) {
        char msg[SFZ_PATH_MAX + 128];
        snprintf(msg, sizeof msg, fmt, arg ? arg : "");
        if (line > 0)
            snprintf(p->err, p->errlen, "%s:%d: %s", p->path, line, msg);
        else
            snprintf(p->err, p->errlen, "%s: %s", p->path, msg);
    }
    return -1;
}

/* A MIDI note: a number, or a name like c4, c#4, db4, a-1 (c4 = 60, the
 * SFZ convention). Returns 0 on success. */
static int parse_note(const char *v, int *out)
{
    static const int semis[7] = {9, 11, 0, 2, 4, 5, 7}; /* a b c d e f g */
    char *end;
    long n = strtol(v, &end, 10);
    int semi, oct;
    if (end != v && *end == '\0') {
        *out = (int)n;
        return 0;
    }
    if (tolower((unsigned char)v[0]) < 'a' || tolower((unsigned char)v[0]) > 'g')
        return -1;
    semi = semis[tolower((unsigned char)v[0]) - 'a'];
    v++;
    if (*v == '#') {
        semi++;
        v++;
    } else if (*v == 'b') {
        semi--;
        v++;
    }
    oct = (int)strtol(v, &end, 10);
    if (end == v || *end != '\0')
        return -1;
    *out = (oct + 1) * 12 + semi;
    return 0;
}

static int parse_int(const char *v, int *out)
{
    char *end;
    long n = strtol(v, &end, 10);
    if (end == v || *end != '\0')
        return -1;
    *out = (int)n;
    return 0;
}

static int parse_float(const char *v, float *out)
{
    char *end;
    double d = strtod(v, &end);
    if (end == v || *end != '\0')
        return -1;
    *out = (float)d;
    return 0;
}

/* Applies one opcode to the current scope. */
static int sfz_opcode(SfzParser *p, int line, const char *name, const char *val)
{
    SfzScope *sc;
    StRegion *r;
    int bad = 0;

    if (p->level == -1) { /* <control> */
        if (strcmp(name, "default_path") == 0) {
            if (strlen(val) >= SFZ_PATH_MAX)
                return sfz_fail(p, line, "default_path too long", NULL);
            strcpy(p->default_path, val);
        }
        return 0;
    }
    if (p->level < 0) /* under an unsupported header */
        return 0;

    sc = &p->scope[p->level];
    r = &sc->r;
    if (strcmp(name, "sample") == 0) {
        if (strlen(val) >= SFZ_PATH_MAX)
            return sfz_fail(p, line, "sample path too long: %s", val);
        strcpy(sc->sample, val);
        sc->set |= OP_SAMPLE;
    } else if (strcmp(name, "lokey") == 0) {
        bad = parse_note(val, &r->lokey);
        sc->set |= OP_LOKEY;
    } else if (strcmp(name, "hikey") == 0) {
        bad = parse_note(val, &r->hikey);
        sc->set |= OP_HIKEY;
    } else if (strcmp(name, "pitch_keycenter") == 0) {
        bad = parse_note(val, &r->keycenter);
        sc->set |= OP_KEYCENTER;
    } else if (strcmp(name, "key") == 0) { /* key= is all three at once */
        bad = parse_note(val, &r->lokey);
        r->hikey = r->keycenter = r->lokey;
        sc->set |= OP_LOKEY | OP_HIKEY | OP_KEYCENTER;
    } else if (strcmp(name, "lovel") == 0) {
        bad = parse_int(val, &r->lovel);
        sc->set |= OP_LOVEL;
    } else if (strcmp(name, "hivel") == 0) {
        bad = parse_int(val, &r->hivel);
        sc->set |= OP_HIVEL;
    } else if (strcmp(name, "tune") == 0) {
        bad = parse_float(val, &r->tune_cents);
        sc->set |= OP_TUNE;
    } else if (strcmp(name, "volume") == 0) {
        bad = parse_float(val, &r->volume_db);
        sc->set |= OP_VOLUME;
    } else if (strcmp(name, "loop_mode") == 0) {
        if (strcmp(val, "loop_continuous") == 0 ||
            strcmp(val, "loop_sustain") == 0)
            r->loop = 1; /* both loop through the release here */
        else if (strcmp(val, "no_loop") == 0 || strcmp(val, "one_shot") == 0)
            r->loop = 0;
        else
            bad = 1;
        sc->set |= OP_LOOP;
    } else if (strcmp(name, "loop_start") == 0) {
        bad = parse_int(val, &r->loop_start);
        sc->set |= OP_LOOP_START;
    } else if (strcmp(name, "loop_end") == 0) {
        bad = parse_int(val, &r->loop_end);
        sc->set |= OP_LOOP_END;
    } else if (strcmp(name, "offset") == 0) {
        bad = parse_int(val, &r->offset);
        sc->set |= OP_OFFSET;
    } else if (strcmp(name, "seq_length") == 0) {
        bad = parse_int(val, &r->seq_length);
        sc->set |= OP_SEQ_LENGTH;
    } else if (strcmp(name, "seq_position") == 0) {
        bad = parse_int(val, &r->seq_position);
        sc->set |= OP_SEQ_POSITION;
    } /* anything else: an opcode this subset does not play; ignored */
    if (bad)
        return sfz_fail(p, line, "bad value for opcode: %s", name);
    return 0;
}

/* Copies the opcodes `src` set over `dst`. */
static void scope_merge(SfzScope *dst, const SfzScope *src)
{
    const StRegion *s = &src->r;
    StRegion *d = &dst->r;
    if (src->set & OP_LOKEY) d->lokey = s->lokey;
    if (src->set & OP_HIKEY) d->hikey = s->hikey;
    if (src->set & OP_KEYCENTER) d->keycenter = s->keycenter;
    if (src->set & OP_LOVEL) d->lovel = s->lovel;
    if (src->set & OP_HIVEL) d->hivel = s->hivel;
    if (src->set & OP_TUNE) d->tune_cents = s->tune_cents;
    if (src->set & OP_VOLUME) d->volume_db = s->volume_db;
    if (src->set & OP_LOOP) d->loop = s->loop;
    if (src->set & OP_LOOP_START) d->loop_start = s->loop_start;
    if (src->set & OP_LOOP_END) d->loop_end = s->loop_end;
    if (src->set & OP_OFFSET) d->offset = s->offset;
    if (src->set & OP_SEQ_LENGTH) d->seq_length = s->seq_length;
    if (src->set & OP_SEQ_POSITION) d->seq_position = s->seq_position;
    if (src->set & OP_SAMPLE) strcpy(dst->sample, src->sample);
    dst->set |= src->set;
}

static int ends_with_ci(const char *s, const char *suffix)
{
    size_t n = strlen(s), m = strlen(suffix), i;
    if (n < m)
        return 0;
    for (i = 0; i < m; i++)
        if (tolower((unsigned char)s[n - m + i]) != suffix[i])
            return 0;
    return 1;
}

/* Decodes `path` (or finds it already decoded) into the set. */
static int sfz_sample(SfzParser *p, int line, const char *path, int *buffer,
                      int *frames, int *rate)
{
    unsigned int channels = 0, sr = 0;
    drflac_uint64 total = 0;
    float *pcm, *mono;
    drflac_uint64 i;
    unsigned c;
    int k;

    for (k = 0; k < p->ncache; k++) {
        if (strcmp(p->cache_path[k], path) == 0) {
            *buffer = p->cache_buffer[k];
            *frames = p->cache_frames[k];
            *rate = p->cache_rate[k];
            return 0;
        }
    }
    if (!ends_with_ci(path, ".flac"))
        return sfz_fail(p, line, "only FLAC samples are supported: %s", path);
    pcm = drflac_open_file_and_read_pcm_frames_f32(path, &channels, &sr,
                                                    &total, NULL);
    if (!pcm)
        return sfz_fail(p, line, "cannot read FLAC sample: %s", path);
    if (channels == 0 || sr == 0 || total == 0 || total > 0x7fffffff) {
        drflac_free(pcm, NULL);
        return sfz_fail(p, line, "empty or oversized FLAC sample: %s", path);
    }
    mono = malloc((size_t)total * sizeof(float));
    if (!mono) {
        drflac_free(pcm, NULL);
        return sfz_fail(p, line, "out of memory decoding %s", path);
    }
    /* Average the channels: VSCO is mono already, but a stereo file
     * still plays (centred; the mixer places the instrument). */
    for (i = 0; i < total; i++) {
        float acc = 0.0f;
        for (c = 0; c < channels; c++)
            acc += pcm[i * channels + c];
        mono[i] = acc / (float)channels;
    }
    drflac_free(pcm, NULL);
    if (set_adopt_buffer(p->set, mono) != 0) {
        free(mono);
        return sfz_fail(p, line, "out of memory decoding %s", path);
    }

    if (p->ncache == p->cache_cap) {
        int cap = p->cache_cap ? p->cache_cap * 2 : 16;
        void *a = realloc(p->cache_path, (size_t)cap * sizeof *p->cache_path);
        if (a) p->cache_path = a;
        a = a ? realloc(p->cache_buffer, (size_t)cap * sizeof(int)) : NULL;
        if (a) p->cache_buffer = a;
        a = a ? realloc(p->cache_frames, (size_t)cap * sizeof(int)) : NULL;
        if (a) p->cache_frames = a;
        a = a ? realloc(p->cache_rate, (size_t)cap * sizeof(int)) : NULL;
        if (a) p->cache_rate = a;
        if (!a)
            return sfz_fail(p, line, "out of memory", NULL);
        p->cache_cap = cap;
    }
    strcpy(p->cache_path[p->ncache], path);
    p->cache_buffer[p->ncache] = p->set->nbuffers - 1;
    p->cache_frames[p->ncache] = (int)total;
    p->cache_rate[p->ncache] = (int)sr;
    p->ncache++;
    *buffer = p->set->nbuffers - 1;
    *frames = (int)total;
    *rate = (int)sr;
    return 0;
}

/* Closes the open <region>: merges global < master < group < region over
 * the defaults, loads its sample and adds it to the set. */
static int sfz_flush_region(SfzParser *p)
{
    SfzScope m;
    char full[3 * SFZ_PATH_MAX];
    const char *problem;
    int lv, buffer = 0, frames = 0, rate = 0;
    char *c;

    if (!p->in_region)
        return 0;
    p->in_region = 0;

    memset(&m, 0, sizeof m);
    m.r.lokey = 0;
    m.r.hikey = 127;
    m.r.keycenter = 60;
    m.r.lovel = 1;
    m.r.hivel = 127;
    m.r.seq_length = 1;
    m.r.seq_position = 1;
    for (lv = 0; lv < LV_COUNT; lv++)
        scope_merge(&m, &p->scope[lv]);
    if (!(m.set & OP_SAMPLE))
        return sfz_fail(p, p->region_line, "region has no sample=", NULL);

    /* An absolute sample path stands alone; a relative one starts at
     * <control> default_path= (itself relative to the map's directory
     * unless absolute). SFZ files often use
     * '\' separators, so those become '/'. */
    if (m.sample[0] == '/')
        snprintf(full, sizeof full, "%s", m.sample);
    else if (p->default_path[0] == '/')
        snprintf(full, sizeof full, "%s%s", p->default_path, m.sample);
    else
        snprintf(full, sizeof full, "%s%s%s", p->dir, p->default_path,
                 m.sample);
    for (c = full; *c; c++)
        if (*c == '\\')
            *c = '/';

    if (sfz_sample(p, p->region_line, full, &buffer, &frames, &rate) != 0)
        return -1;
    m.r.data = p->set->buffers[buffer];
    m.r.frames = frames;
    m.r.sample_rate = rate;
    problem = region_problem(&m.r);
    if (problem) {
        char msg[3 * SFZ_PATH_MAX + 64];
        snprintf(msg, sizeof msg, "%s (%s)", problem, full);
        return sfz_fail(p, p->region_line, "%s", msg);
    }
    if (set_push_region(p->set, &m.r) < 0)
        return sfz_fail(p, p->region_line, "out of memory", NULL);
    return 0;
}

static int sfz_header(SfzParser *p, int line, const char *name)
{
    int lv;
    if (sfz_flush_region(p) != 0)
        return -1;
    if (strcmp(name, "control") == 0) {
        p->level = -1;
        return 0;
    }
    if (strcmp(name, "global") == 0)
        lv = LV_GLOBAL;
    else if (strcmp(name, "master") == 0)
        lv = LV_MASTER;
    else if (strcmp(name, "group") == 0)
        lv = LV_GROUP;
    else if (strcmp(name, "region") == 0)
        lv = LV_REGION;
    else { /* <curve>, <effect>, ...: nothing this subset plays */
        p->level = -2;
        return 0;
    }
    /* A header starts a fresh scope at its level and closes the ones
     * inside it (a new <group> forgets the last group's opcodes). */
    memset(&p->scope[lv], 0, (size_t)(LV_COUNT - lv) * sizeof p->scope[0]);
    p->level = lv;
    if (lv == LV_REGION) {
        p->in_region = 1;
        p->region_line = line;
    }
    return 0;
}

/* Is s at the start of an opcode ("name=")? */
static int at_opcode(const char *s)
{
    const char *q = s;
    while (isalnum((unsigned char)*q) || *q == '_')
        q++;
    return q > s && *q == '=';
}

/* One line, comment already cut off. */
static int sfz_line(SfzParser *p, int line, char *s)
{
    for (;;) {
        while (isspace((unsigned char)*s))
            s++;
        if (!*s)
            return 0;
        if (*s == '<') {
            char *close = strchr(s, '>');
            if (!close)
                return sfz_fail(p, line, "unclosed header", NULL);
            *close = '\0';
            if (sfz_header(p, line, s + 1) != 0)
                return -1;
            s = close + 1;
        } else if (at_opcode(s)) {
            char *eq = strchr(s, '='), *val = eq + 1, *end;
            *eq = '\0';
            if (strcmp(s, "sample") == 0 || strcmp(s, "default_path") == 0) {
                /* Paths may hold spaces: the value runs up to the next
                 * opcode or header on the line. */
                end = val;
                while (*end) {
                    if (isspace((unsigned char)*end)) {
                        char *q = end;
                        while (isspace((unsigned char)*q))
                            q++;
                        if (!*q || *q == '<' || at_opcode(q))
                            break;
                        end = q;
                    } else if (*end == '<') {
                        break;
                    } else {
                        end++;
                    }
                }
            } else {
                end = val;
                while (*end && !isspace((unsigned char)*end) && *end != '<')
                    end++;
            }
            {
                char saved = *end;
                *end = '\0';
                if (sfz_opcode(p, line, s, val) != 0)
                    return -1;
                *end = saved;
            }
            s = end;
        } else {
            return sfz_fail(p, line, "cannot parse: %s", s);
        }
    }
}

StSampleSet *st_sfz_load(const char *sfz_path, char *err, size_t errlen)
{
    SfzParser *p;
    FILE *f;
    char *text = NULL, *s, *next;
    long size;
    int line = 1, ok = 0;
    const char *slash;

    if (err && errlen)
        err[0] = '\0';
    p = calloc(1, sizeof *p);
    if (!p) {
        if (err && errlen)
            snprintf(err, errlen, "%s: out of memory", sfz_path);
        return NULL;
    }
    p->path = sfz_path;
    p->err = err;
    p->errlen = errlen;
    p->level = -2; /* opcodes before any header are ignored */

    slash = strrchr(sfz_path, '/');
    {
        const char *bs = strrchr(sfz_path, '\\');
        if (bs && (!slash || bs > slash))
            slash = bs;
    }
    if (slash) {
        size_t n = (size_t)(slash - sfz_path) + 1;
        if (n >= SFZ_PATH_MAX) {
            sfz_fail(p, 0, "path too long", NULL);
            goto done;
        }
        memcpy(p->dir, sfz_path, n);
        p->dir[n] = '\0';
    }

    f = fopen(sfz_path, "rb");
    if (!f) {
        sfz_fail(p, 0, "cannot open the map", NULL);
        goto done;
    }
    if (fseek(f, 0, SEEK_END) != 0 || (size = ftell(f)) < 0 ||
        fseek(f, 0, SEEK_SET) != 0) {
        fclose(f);
        sfz_fail(p, 0, "cannot read the map", NULL);
        goto done;
    }
    text = malloc((size_t)size + 1);
    if (!text || fread(text, 1, (size_t)size, f) != (size_t)size) {
        fclose(f);
        sfz_fail(p, 0, "cannot read the map", NULL);
        goto done;
    }
    fclose(f);
    text[size] = '\0';

    p->set = st_sampleset_new();
    if (!p->set) {
        sfz_fail(p, 0, "out of memory", NULL);
        goto done;
    }
    for (s = text; s; s = next, line++) {
        char *cut;
        next = strchr(s, '\n');
        if (next)
            *next++ = '\0';
        cut = strstr(s, "//");
        if (cut)
            *cut = '\0';
        cut = strchr(s, '\r');
        if (cut)
            *cut = '\0';
        if (sfz_line(p, line, s) != 0)
            goto done;
    }
    if (sfz_flush_region(p) != 0)
        goto done;
    if (p->set->nregions == 0) {
        sfz_fail(p, 0, "no <region> in the map", NULL);
        goto done;
    }
    ok = 1;

done:
    free(text);
    free(p->cache_path);
    free(p->cache_buffer);
    free(p->cache_frames);
    free(p->cache_rate);
    {
        StSampleSet *set = p->set;
        free(p);
        if (!ok) {
            st_sampleset_free(set);
            return NULL;
        }
        return set;
    }
}

/* ================================================================== */
/* The voice                                                           */

/* Shortest release, so a patch with release 0 still ends without a
 * click (2 ms is below what reads as a fade). */
#define MIN_RELEASE 0.002f
/* The last quarter of the release also fades linearly to exactly zero.
 * The shared envelope (studio.h) covers 99% of its way in `release` but
 * only reaches its 1e-5 end ~2.5x later; this fade ends the note at the
 * release time, from about -30 dB, where the extra slope is inaudible. */
#define RELEASE_TAIL 0.25f

/* The first region matching key and velocity whose round-robin slot is
 * `rr`. When a match exists but no slot fits (a map whose seq_positions
 * leave holes), the first match plays rather than nothing. Only one
 * region plays per copy: this subset does not layer regions. */
static const StRegion *pick_region(const StSampleSet *set, int key, int vel,
                                   unsigned rr)
{
    const StRegion *first = NULL;
    int i;
    for (i = 0; i < set->nregions; i++) {
        const StRegion *r = &set->regions[i];
        if (key < r->lokey || key > r->hikey || vel < r->lovel ||
            vel > r->hivel)
            continue;
        if (!first)
            first = r;
        if ((int)(rr % (unsigned)r->seq_length) + 1 == r->seq_position)
            return r;
    }
    return first;
}

int st_sampler_start(StSamplerVoice *v, const StSampleSet *set,
                     const StSamplerPatch *patch, float rate, float note,
                     int velocity, unsigned rr, uint32_t seed)
{
    int stack, i, key;
    float curve, vel_gain, norm;
    int order[ST_SAMPLER_MAX_STACK];
    uint32_t rng = seed ? seed : 0x5EED5EEDu;

    memset(v, 0, sizeof *v);
    if (!set || !patch || rate <= 0.0f)
        return 0;
    if (velocity < 1) velocity = 1;
    if (velocity > 127) velocity = 127;
    key = (int)floorf(note + 0.5f);
    stack = patch->stack < 1 ? 1 : patch->stack;
    if (stack > ST_SAMPLER_MAX_STACK)
        stack = ST_SAMPLER_MAX_STACK;

    curve = patch->velocity_curve > 0.0f ? patch->velocity_curve : 1.7f;
    vel_gain = powf((float)velocity / 127.0f, curve);
    /* Uncorrelated copies add in power: 1/sqrt(stack) keeps the level. */
    norm = 1.0f / sqrtf((float)stack);

    /* Which copy starts when: copy i (by detune and pan) takes onset slot
     * order[i], shuffled by the seed so the lowest-detuned copy is not
     * always the first to speak (Fisher-Yates). */
    for (i = 0; i < stack; i++)
        order[i] = i;
    for (i = stack - 1; i > 0; i--) {
        int j = (int)(st_rand(&rng) % (uint32_t)(i + 1));
        int t = order[i];
        order[i] = order[j];
        order[j] = t;
    }

    v->amp = patch->amp;
    if (v->amp.release < MIN_RELEASE)
        v->amp.release = MIN_RELEASE;
    v->rate = rate;
    v->release_frames = (int)(v->amp.release * rate + 0.5f);
    if (v->release_frames < 1)
        v->release_frames = 1;

    for (i = 0; i < stack; i++) {
        StSamplerHead *h = &v->head[i];
        const StRegion *r = pick_region(set, key, velocity, rr + (unsigned)i);
        float t = stack > 1 ? (float)i / (float)(stack - 1) : 0.5f;
        float detune = stack > 1 ? (t - 0.5f) * patch->stack_detune_cents : 0.0f;
        double semis;
        long start;
        if (!r) {
            memset(v, 0, sizeof *v);
            return 0;
        }
        h->region = r;
        semis = (double)note - r->keycenter +
                (r->tune_cents + patch->tune_cents + detune) / 100.0;
        h->inc_base = pow(2.0, semis / 12.0) * r->sample_rate / rate;
        h->gain = st_db_to_gain(r->volume_db) * vel_gain * norm;
        if (stack == 1) {
            /* Unstacked: centred, the same signal on both sides at unity;
             * the mixer pans the instrument. */
            h->pan_l = h->pan_r = 1.0f;
        } else {
            /* Equal-power pan across the spread, scaled by sqrt(2) so a
             * centred copy has unity gain on each side like stack 1. */
            float spread = patch->stereo_spread < 0.0f ? 0.0f
                         : patch->stereo_spread > 1.0f ? 1.0f
                         : patch->stereo_spread;
            float pos = (2.0f * t - 1.0f) * spread;          /* -1..1 */
            float angle = (pos + 1.0f) * 0.78539816f;        /* 0..pi/2 */
            h->pan_l = 1.41421356f * cosf(angle);
            h->pan_r = 1.41421356f * sinf(angle);
        }
        h->delay = stack > 1
            ? (int)(patch->stack_delay * rate * (float)order[i] /
                        (float)(stack - 1) + 0.5f)
            : 0;
        if (h->delay < 0)
            h->delay = 0;
        start = r->offset +
                (long)(patch->start_offset * (float)r->sample_rate + 0.5f);
        h->pos = (double)(start < 0 ? 0 : start);
        h->release_in = -1;
        h->rel_count = -1;
        if (r->loop) {
            /* A start offset past the loop lands inside it, as if the
             * playhead had already gone round. */
            while (h->pos >= (double)r->loop_end + 1.0) {
                h->pos -= (double)r->loop_end - r->loop_start + 1.0;
                h->wrapped = 1;
            }
        } else if (h->pos >= r->frames) {
            h->done = 1; /* starts past the end of a one-shot: silent */
        }
    }
    v->nheads = stack;
    v->active = 1;
    for (i = 0; i < stack; i++)
        if (!v->head[i].done)
            return 1;
    v->active = 0; /* every copy started past its sample's end */
    return 1;
}

void st_sampler_release(StSamplerVoice *v)
{
    int i;
    if (!v->active || v->released)
        return;
    v->released = 1;
    for (i = 0; i < v->nheads; i++) {
        StSamplerHead *h = &v->head[i];
        if (h->done)
            continue;
        if (h->started) {
            st_env_off(&h->env);
            h->rel_count = 0;
        } else {
            /* Not yet speaking (a stacked copy's onset delay): it still
             * sounds, held as long as the note was, then released. */
            h->release_in = (int)v->elapsed;
        }
    }
}

int st_sampler_active(const StSamplerVoice *v)
{
    return v->active;
}

/* One sample of the region at integer index i. Past loop_end a looping
 * playhead reads the loop's start (the continuation it will play), and
 * once it has wrapped, before loop_start it reads the loop's end, so the
 * 4-tap interpolation never straddles the seam into unlooped audio.
 * Outside the sample it reads silence. */
static float tap(const StRegion *r, long i, int wrapped)
{
    if (r->loop) {
        long len = (long)r->loop_end - r->loop_start + 1;
        if (i > r->loop_end)
            i -= len;
        else if (wrapped && i < r->loop_start)
            i += len;
    }
    if (i < 0 || i >= r->frames)
        return 0.0f;
    return r->data[i];
}

/* 4-point Catmull-Rom between x0 and x1, t in [0, 1). */
static float interp(const StRegion *r, double pos, int wrapped)
{
    long i = (long)pos; /* pos >= 0 */
    float t = (float)(pos - (double)i);
    float xm1 = tap(r, i - 1, wrapped), x0 = tap(r, i, wrapped);
    float x1 = tap(r, i + 1, wrapped), x2 = tap(r, i + 2, wrapped);
    return x0 + 0.5f * t * (x1 - xm1 +
                            t * (2.0f * xm1 - 5.0f * x0 + 4.0f * x1 - x2 +
                                 t * (3.0f * (x0 - x1) + x2 - xm1)));
}

int st_sampler_render(StSamplerVoice *v, const StVoiceMod *mod, float *l,
                      float *r, int n)
{
    float g0 = mod ? mod->gain0 : 1.0f, g1 = mod ? mod->gain1 : 1.0f;
    float pitch = mod ? mod->pitch_cents : 0.0f;
    double bend = pitch != 0.0f ? pow(2.0, pitch / 1200.0) : 1.0;
    float dg = n > 0 ? (g1 - g0) / (float)n : 0.0f;
    float tail = RELEASE_TAIL * (float)v->release_frames;
    int i, k, sounding = 0;

    if (!v->active)
        return 0;
    for (i = 0; i < v->nheads; i++) {
        StSamplerHead *h = &v->head[i];
        const StRegion *reg = h->region;
        double inc = h->inc_base * bend;
        double loop_end1 = (double)reg->loop_end + 1.0;
        double loop_len = (double)reg->loop_end - reg->loop_start + 1.0;

        for (k = 0; k < n && !h->done; k++) {
            float env, s, fade = 1.0f;
            if (h->delay > 0) {
                h->delay--;
                continue;
            }
            if (!h->started) {
                h->started = 1;
                st_env_on(&h->env);
            }
            if (h->release_in == 0) {
                st_env_off(&h->env);
                h->rel_count = 0;
            }
            if (h->release_in >= 0)
                h->release_in--;
            env = st_env_next(&h->env, &v->amp, v->rate);
            if (h->rel_count >= 0) {
                float left = (float)(v->release_frames - h->rel_count);
                if (left <= 0.0f || h->env.stage == 0) {
                    h->done = 1;
                    break;
                }
                if (left < tail)
                    fade = left / tail;
                h->rel_count++;
            }
            s = interp(reg, h->pos, h->wrapped) * env * fade * h->gain *
                (g0 + dg * (float)k);
            l[k] += s * h->pan_l;
            r[k] += s * h->pan_r;

            h->pos += inc;
            if (reg->loop) {
                while (h->pos >= loop_end1) {
                    h->pos -= loop_len; /* keeps the fractional overshoot */
                    h->wrapped = 1;
                }
            } else if (h->pos >= reg->frames) {
                h->done = 1;
            }
        }
        if (!h->done)
            sounding = 1;
    }
    v->elapsed += n;
    if (!sounding)
        v->active = 0;
    return sounding;
}
