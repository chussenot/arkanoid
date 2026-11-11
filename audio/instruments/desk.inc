# The mixing desk every BRYX score uses (docs/design/orchestral-score.md, 4.1).
# Include it first, then orchestra.inc and electronic.inc.

# Aux returns: the four spaces, and a tempo-synced ping-pong echo.
aux room  reverb preset=room
aux stage reverb preset=stage
aux hall  reverb preset=hall
aux long  reverb preset=long
aux echo  delay time=0.75 time_r=0.5 feedback=0.35 pingpong=1 lowcut=250 highcut=5000 gain=-3

# Group buses. The electronic bus leaves the middle to the orchestra and
# the gameplay sounds (TRON: Legacy's synths kept to the low pulse and the
# high sparkle): a broad cut around 1.2 kHz, and nothing below 28 Hz.
# The orchestra bus lifts presence and air: VSCO's recordings are dark
# (a violin section's 8-12 kHz sits near -40 dB under its body, against
# -29 dB for the synths), and folding them to mono costs ~2 dB more. The
# sampler and the reverb are not the cause (docs/design/studio-report.md,
# 5); the reverb's damping moved the mix by 0.1 dB, so it stays as it is.
bus orchestra  gain=0 peak=3200:0.9:2 highshelf=7000:4
bus electronic gain=-1 hpf=28 peak=1200:0.6:-3
bus percussion gain=0 hpf=30

# A gentle glue compressor and a ceiling: dynamics stay, peaks never clip.
master comp=-16:1.6:0.03:0.35 limit=-1
