/*
 * studio_sampler.h: the studio's SFZ-subset sampler (BRYX addition; see
 * studio.h and docs/design/orchestral-score.md, 4.2-4.3).
 *
 * Two halves:
 *   - StSampleSet: an instrument's regions and the mono float audio they
 *     play, built from an .sfz map (st_sfz_load) or by hand
 *     (st_sampleset_add_region, used by the tests). All allocation and
 *     file I/O happen here, at load time.
 *   - StSamplerVoice: one sounding note, a plain struct with no heap. It
 *     plays up to ST_SAMPLER_MAX_STACK copies ("playheads") of the note,
 *     each on its own region, to turn a solo recording into a section.
 *
 * A voice keeps pointers into its set, so the set must outlive every
 * voice started on it, and no region may be added to a set once voices
 * play it (adding can move the region array).
 */
#ifndef BRYX_STUDIO_SAMPLER_H
#define BRYX_STUDIO_SAMPLER_H

#include <stddef.h>
#include <stdint.h>

#include "studio.h"

/* One SFZ <region>. */
typedef struct {
    int lokey, hikey, keycenter, lovel, hivel; /* MIDI 0..127, vel 1..127 */
    float tune_cents;         /* tune= */
    float volume_db;          /* volume= */
    int loop;                 /* 1 for loop_mode=loop_continuous or loop_sustain */
    int loop_start, loop_end; /* frames, inclusive end as in SFZ */
    int offset;               /* offset= frames */
    int seq_length, seq_position; /* round robin, default 1/1 */
    const float *data;        /* mono float samples, owned by the set */
    int frames;
    int sample_rate;
} StRegion;

typedef struct StSampleSet StSampleSet; /* opaque */

StSampleSet *st_sampleset_new(void);
/* Copies `data` (mono, `frames` long) and adds a region playing it, with
 * every field but data/frames/sample_rate taken from r_template. Returns
 * the region index, or -1 when the template is invalid (bad ranges, a
 * loop outside the sample) or memory runs out. */
int st_sampleset_add_region(StSampleSet *s, const StRegion *r_template,
                            const float *data, int frames, int sample_rate);
/* Parses an SFZ-subset map; sample= paths are relative to the .sfz
 * file's directory (plus <control> default_path=). Headers: <control>,
 * <global>, <master>, <group>, <region>; opcodes from a header are
 * inherited by the regions after it (global < master < group < region).
 * Opcodes: sample lokey hikey key pitch_keycenter tune volume lovel hivel
 * loop_mode loop_start loop_end offset seq_length seq_position; key
 * values may be numbers or note names (c4 = 60). Unknown opcodes and the
 * opcodes under unknown headers are ignored. `//` starts a comment.
 * Samples must be FLAC (any channel count is averaged to mono); a file
 * named by several regions is decoded once. Returns NULL and writes a
 * message to err (when err/errlen allow) on any failure. */
StSampleSet *st_sfz_load(const char *sfz_path, char *err, size_t errlen);
void st_sampleset_free(StSampleSet *s);
int st_sampleset_region_count(const StSampleSet *s);
const StRegion *st_sampleset_region(const StSampleSet *s, int i);

#define ST_SAMPLER_MAX_STACK 4

/* One copy of the note: a playhead on one region. */
typedef struct {
    const StRegion *region;
    double pos;          /* frames into region->data */
    double inc_base;     /* frames per output frame before mod->pitch_cents */
    float gain;          /* region volume x velocity x 1/sqrt(stack) */
    float pan_l, pan_r;  /* per-side gains (1, 1 for an unstacked voice) */
    int delay;           /* output frames left before this copy starts */
    int started;         /* 1 once the onset delay has run out */
    int wrapped;         /* 1 once the playhead has looped at least once */
    int release_in;      /* -1, or frames after onset until note-off */
    int rel_count;       /* frames since this copy's note-off, -1 before */
    int done;
    StEnv env;
} StSamplerHead;

typedef struct {
    StSamplerHead head[ST_SAMPLER_MAX_STACK];
    int nheads;
    StAdsr amp;          /* the patch's envelope, release floored (de-click) */
    float rate;          /* output sample rate */
    int release_frames;  /* the release length in output frames */
    long elapsed;        /* output frames rendered since the start */
    int released;
    int active;
} StSamplerVoice;

/* Starts a note (fractional notes bend the pitch; velocity 1..127). `rr`
 * is a per-instrument counter the caller increments each note: among the
 * regions matching key and velocity, the one with seq_position ==
 * (rr % seq_length) + 1 plays; stacked copy i uses rr + i. Returns 0 when
 * no region matches (the voice stays inactive), else 1. `seed` drives the
 * only randomness (the order of the stacked copies' onsets). */
int st_sampler_start(StSamplerVoice *v, const StSampleSet *set,
                     const StSamplerPatch *patch, float rate, float note,
                     int velocity, unsigned rr, uint32_t seed);
/* Note-off: every copy enters its release. */
void st_sampler_release(StSamplerVoice *v);
/* ADDs n stereo frames into l/r. mod may be NULL (unity gain, no bend).
 * Returns 1 while sounding, 0 once finished (release done or every
 * one-shot copy past its sample's end). */
int st_sampler_render(StSamplerVoice *v, const StVoiceMod *mod, float *l,
                      float *r, int n);
int st_sampler_active(const StSamplerVoice *v);

#endif
