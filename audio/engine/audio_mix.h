#ifndef ELECTRIC_PULSE_AUDIO_MIX_H
#define ELECTRIC_PULSE_AUDIO_MIX_H

#include "audio_seq.h"

unsigned char *audio_mix_render_song(const SeqSong *song, int sample_rate, int *out_len);
unsigned char *audio_mix_render_timeline(const SeqSong *song, const SeqTimeline *timeline,
                                         int sample_rate, int *out_len);
/* BRYX: the same render as signed 16-bit samples (free() the buffer). */
int16_t *audio_mix_render_song_s16(const SeqSong *song, int sample_rate, int *out_len);
int16_t *audio_mix_render_timeline_s16(const SeqSong *song, const SeqTimeline *timeline,
                                       int sample_rate, int *out_len);

#endif
