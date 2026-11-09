#!/usr/bin/env python3
"""The credits' song: "The War Reborn" by Hadal Static (used with permission; see
assets/reborn/music/NOTICE.txt). From the band's master (a WAV, not in the repository) this writes
the song the game ships, assets/reborn/music/the_war_reborn.ogg (Ogg Vorbis, quality 8), and its
cue sheet, apps/ta_game/credits/song.toml, which the credits' film (credits_show.rs) is timed to:

* every bar's first beat, tracked through the whole song (its tempo drifts from 135 to 141 BPM):
  dynamic programming over the onset envelope (Ellis 2007), the bar's first beat where the band
  comes in, the break falls and the finale hits (the launch trailer's grid);
* the song's moments the film lands on: the band coming in, the break, the drop after it, the
  loudest bar, the bar after it, the flash, the dead stop;
* the song made longer for a film that needs it ("arrangements"): the film is about twice the
  song, so some of its bars play again. The song has no bar-for-bar repeats, but its two long
  sections (B, bars 20-46, and C after the drop, 48-71) share their riff and chords: a jump back
  from a bar of C into B (or within C, or from B into A) can land where the music is as it would
  have been. Every jump back between two bar starts is scored at its seam (the spectrum, the bass
  note and the chord just before against just after, each as a percentile of the same change at
  the song's own bar lines) and over the two bars around it (what is heard after the seam against
  what it replaces, and what was heard before against what leads into it); the jumps that pass
  (JUMPS) are aligned to the sample on their downbeats (the incoming bar's rhythm against the
  outgoing one's), and joined by a short crossfade ending just before the downbeat's kick, which
  then plays whole. The arrangements, shortest first, string them together.

    python tools/credits/song.py <master.wav> [--preview <dir>] [--no-ogg]

(From the repository root; ffmpeg on the PATH, numpy.) --preview renders every arrangement as the
game plays it (WAV) and prints each seam's figures.
"""
import argparse
import json
import math
import os
import subprocess
import sys

import numpy as np

ROOT = os.path.normpath(os.path.join(os.path.dirname(__file__), '..', '..'))
OGG = os.path.join(ROOT, 'assets', 'reborn', 'music', 'the_war_reborn.ogg')
CUE = os.path.join(ROOT, 'apps', 'ta_game', 'credits', 'song.toml')
# The path the game reads it by (assets/reborn is mounted over the install).
GAME_PATH = 'music/the_war_reborn.ogg'

RATE = 48000
# The tempo the beat tracker expects (the launch trailer's measure), and the place of the band's
# entry (the bar's first beat there).
BPM = 135.95
BAND_NEAR = 5.40
# The moments the film lands on (seconds; measured for the launch trailer, which cut on them).
MOMENTS = {
    'band': 5.383,      # the band comes in
    'break': 82.57,     # the break: near silence and a riser
    'drop': 84.17,      # the drop after it: the strongest hit
    'loudest': None,    # the loudest bar: bar 72's first beat (filled in from the bars)
    'after': None,      # bar 73's
    'flash': 131.47,    # the flash: the finale comes back in
    'stop': 140.3,      # the song stops dead
}
LOUDEST_BAR = 72
# The crossfade at a seam (seconds) and how far before the incoming downbeat's kick it ends.
CROSSFADE = 0.012
BEFORE_KICK = 0.003
# The playback gain against the master (-3.5 dB: the trailer's level; the master is mastered at
# -10.5 LUFS, TA's own music at -13 to -17).
GAIN_DB = -3.5
# The jumps back the arrangements are made of: (from bar, back to bar), found by `scores()` and
# `match()`: each passes every seam check (spectrum, bass and chord at the seam at most the 50th
# percentile of the song's own bar changes; the two bars either side alike) and the two passages'
# low end (kick and bass) and onsets line up once aligned (`match`'s r: 0.7 and more within C,
# where eight bars come round again nearly as they were; 0.3 from C back into B, which shares its
# riff and chords: an edit on the downbeat, the same chord either side).
JUMPS = [
    (65, 61),  # within C: four bars again
    (65, 57),  # within C: eight bars again
    (66, 57),  # within C: nine
    (39, 31),  # within B: eight bars again
    (61, 39),  # from C back into B's last phrase: the break and the drop again
    (53, 39),  # ... from earlier in C
    (53, 31),  # ... further back
    (53, 19),  # from C back to B's first hit
    (54, 22),  # from C back into the start of B
]
# A seam is risked only where the passages match this well (`match`, the lower of its two r).
LEAST_MATCH = 0.25


def load(path):
    raw = subprocess.run(['ffmpeg', '-v', 'error', '-i', path, '-map', '0:a', '-ac', '2', '-ar', str(RATE), '-f', 'f32le', '-'],
                         capture_output=True, check=True).stdout
    return np.frombuffer(raw, np.float32).reshape(-1, 2)


# ---- Beats and bars ---------------------------------------------------------------------------

def track(path):
    """Beats and bars as the launch trailer measured them (its edit's grid): the song in mono at
    22.05 kHz, the onset envelope of its log spectrum's rise."""
    sr, hop, win = 22050, 256, 2048
    raw = subprocess.run(['ffmpeg', '-v', 'error', '-i', path, '-map', '0:a', '-ac', '1', '-ar', str(sr), '-f', 'f32le', '-'],
                         capture_output=True, check=True).stdout
    a = np.frombuffer(raw, np.float32)
    n = (len(a) - win) // hop
    idx = np.arange(win)[None, :] + hop * np.arange(n)[:, None]
    spec = np.abs(np.fft.rfft(a[idx] * np.hanning(win), axis=1))
    logspec = np.log1p(spec * 20)
    flux = np.maximum(np.diff(logspec, axis=0, prepend=logspec[:1]), 0)
    onset = flux.sum(axis=1)
    k = 64
    onset = onset - np.convolve(onset, np.ones(k) / k, mode='same')
    onset = np.maximum(onset, 0) / (onset.std() + 1e-9)
    fps = sr / hop
    period = 60 / BPM * fps
    alpha = 400.0
    score = onset.copy()
    back = np.full(n, -1)
    lo, hi = int(round(period / 2)), int(round(period * 2))
    offsets = np.arange(-hi, -lo + 1)
    penalty = -alpha * np.log(-offsets / period) ** 2
    for i in range(hi, n):
        cand = score[i + offsets] + penalty
        j = int(np.argmax(cand))
        if cand[j] > 0:
            score[i] = onset[i] + cand[j]
            back[i] = i + offsets[j]
    tail = np.arange(n - int(period * 2), n)
    i = int(tail[np.argmax(score[tail])])
    beats = []
    while i >= 0:
        beats.append(i)
        i = back[i]
    bt = np.array(beats[::-1]) / fps
    start = int(np.searchsorted(bt, BAND_NEAR - 0.2))
    phase = int(np.argmin([abs(bt[start + p] - BAND_NEAR) for p in range(4)]))
    bars = bt[start + phase::4]
    step = np.diff(bars[:8]).mean()
    pre = [round(bars[0] - k * step, 3) for k in range(int(bars[0] / 1.7) + 1, 0, -1)]
    return [round(float(x), 3) for x in bt], [x for x in pre if x > 0] + [round(float(x), 3) for x in bars]


# ---- Seams -------------------------------------------------------------------------------------

class Features:
    """The song's spectrum (40 log bands), bass notes and chords, frame by frame."""

    def __init__(self, stereo):
        mono = stereo.mean(1)
        self.win, self.hop = 4096, 512
        n = (len(mono) - self.win) // self.hop
        frames = np.lib.stride_tricks.sliding_window_view(mono, self.win)[::self.hop][:n] * np.hanning(self.win).astype(np.float32)
        spec = np.abs(np.fft.rfft(frames, axis=1)).astype(np.float32) ** 2
        freqs = np.fft.rfftfreq(self.win, 1 / RATE)
        edges = np.geomspace(30, 18000, 41)
        self.mel = np.stack([np.log(spec[:, (freqs >= a) & (freqs < b)].sum(1) + 1e-7) for a, b in zip(edges[:-1], edges[1:])], 1)

        def chroma(lo, hi):
            sel = (freqs > lo) & (freqs < hi)
            pc = (np.round(12 * np.log2(freqs[sel] / 440.0)) % 12).astype(int)
            out = np.zeros((n, 12), np.float32)
            sub = spec[:, sel]
            for k in range(12):
                out[:, k] = sub[:, pc == k].sum(1)
            return out / (out.sum(1, keepdims=True) + 1e-12)

        self.bass, self.full, self.n = chroma(35, 250), chroma(100, 3000), n
        self.fps = RATE / self.hop

    def span(self, feat, t0, t1):
        f0, f1 = max(0, int(round(t0 * self.fps))), min(self.n, int(round(t1 * self.fps)))
        return feat[f0:max(f1, f0 + 1)].mean(0)


def cos_d(x, y):
    return 1 - float(np.dot(x, y) / (np.linalg.norm(x) * np.linalg.norm(y) + 1e-12))


def scores(f, bars):
    """Every jump back (from bar j to bar i): its seam's three changes as percentiles of the song's
    own bar changes, and the two bars around it alike or not."""
    w = 0.35

    def seam(t_out, t_in):
        return (float(np.mean(np.abs(f.span(f.mel, t_out - w, t_out) - f.span(f.mel, t_in, t_in + w)))),
                cos_d(f.span(f.bass, t_out - 2 * w, t_out), f.span(f.bass, t_in, t_in + 2 * w)),
                cos_d(f.span(f.full, t_out - 2 * w, t_out), f.span(f.full, t_in, t_in + 2 * w)))

    nb = len(bars) - 1
    natural = np.array([seam(bars[k], bars[k]) for k in range(2, nb - 1)])
    out = {}
    L = 2 * 1.73
    for i in range(3, nb - 2):
        for j in range(i + 2, nb - 1):
            s = seam(bars[j], bars[i])
            pct = [float((natural[:, c] < s[c]).mean() * 100) for c in range(3)]
            after = float(np.mean(np.abs(f.span(f.mel, bars[i], bars[i] + L) - f.span(f.mel, bars[j], bars[j] + L))))
            before = float(np.mean(np.abs(f.span(f.mel, bars[i] - L, bars[i]) - f.span(f.mel, bars[j] - L, bars[j]))))
            ab = cos_d(f.span(f.bass, bars[i], bars[i] + L), f.span(f.bass, bars[j], bars[j] + L))
            bb = cos_d(f.span(f.bass, bars[i] - L, bars[i]), f.span(f.bass, bars[j] - L, bars[j]))
            out[(j, i)] = dict(seam=pct, after=after, before=before, bass_after=ab, bass_before=bb)
    return out


def passes(s):
    return max(s['seam']) <= 50 and s['after'] <= 0.6 and s['before'] <= 0.6 and s['bass_after'] <= 0.12 and s['bass_before'] <= 0.12


class Rhythm:
    """The song's low end (kick and bass: under 160 Hz, as a waveform) and its onsets (the
    spectrum's rise, 1 ms apart), to line two passages up by."""

    def __init__(self, stereo):
        mono = stereo.mean(1).astype(np.float64)
        spec = np.fft.rfft(mono)
        freqs = np.fft.rfftfreq(len(mono), 1 / RATE)
        self.low = np.fft.irfft(spec * (freqs < 160), n=len(mono)).astype(np.float32)
        self.hop, win = 48, 1024
        n = (len(mono) - win) // self.hop
        env = np.zeros(n, np.float32)
        prev = None
        for c0 in range(0, n, 20000):
            c1 = min(n, c0 + 20000)
            idx = np.arange(win)[None, :] + self.hop * np.arange(c0, c1)[:, None]
            s = np.log1p(np.abs(np.fft.rfft(mono[idx] * np.hanning(win), axis=1)) * 10)
            prev = s[:1] if prev is None else prev
            env[c0:c1] = np.maximum(np.diff(np.concatenate([prev, s]), axis=0), 0).sum(1)
            prev = s[-1:]
        self.env = env
        self.win = win


def ncc(x, y):
    x = x - x.mean()
    y = y - y.mean()
    return float(np.dot(x, y) / (np.linalg.norm(x) * np.linalg.norm(y) + 1e-12))


def match(r, t_out, t_in, pre=0.25, post=0.9):
    """How the passage from bar t_in lines up with the one from t_out: the lag (|lag| <= 40 ms) at
    which the low end and the onsets correlate best together, and each one's correlation there."""
    o0 = int((t_out - pre) * RATE)
    xo = r.low[o0:o0 + int((pre + post) * RATE)]
    e0 = int((t_out - pre) * RATE / r.hop)
    eo = r.env[e0:e0 + int((pre + post) * RATE / r.hop)]
    best = (-9.0, 0.0, 0.0, 0.0)
    for lag in np.arange(-0.040, 0.04025, 0.00025):
        i0 = int(round((t_in + lag - pre) * RATE))
        a = ncc(xo, r.low[i0:i0 + len(xo)])
        j0 = int(round((t_in + lag - pre) * RATE / r.hop))
        b = ncc(eo, r.env[j0:j0 + len(eo)])
        if a + b > best[0]:
            best = (a + b, lag, a, b)
    return best[1], best[2], best[3]


def align(stereo, r, t_out, t_in):
    """The seam's two places, to the sample: the incoming bar lined up with the outgoing one
    (`match`), the join just before the incoming downbeat's kick (the strongest rise of the low
    band within 50 ms of the aligned bar line), the outgoing side cut as far before its own."""
    lag, rl, ro = match(r, t_out, t_in)
    aligned_in = t_in + lag
    # The low end's level (2 ms windows) round the aligned bar line; the kick where it rises most
    # over 4 ms.
    a0 = int((aligned_in - 0.05) * RATE)
    seg = r.low[a0:a0 + int(0.1 * RATE)].astype(np.float64)
    w = 96
    level = 10 * np.log10(np.convolve(seg ** 2, np.ones(w) / w, mode='same') + 1e-12)
    d = 192
    rise = level[d:] - level[:-d]
    k = int(np.argmax(rise)) + d // 2
    kick = (a0 + k) / RATE
    seam_in = kick - BEFORE_KICK
    seam_out = t_out + (seam_in - aligned_in)
    return round(seam_out, 6), round(seam_in, 6), lag, rl, ro


# ---- Arrangements -------------------------------------------------------------------------------

def walks(nb, jumps, max_jumps):
    """Every way through the song taking jumps back in turn (each jump's from-bar reached going
    forward from where the last landed), as lists of jumps; the plays of each bar at most four, a
    span between two jumps at least eight bars."""
    out = [[]]

    def grow(path, at):
        if len(path) >= max_jumps:
            return
        for (j, i) in jumps:
            if j - at < 8:
                continue
            new = path + [(j, i)]
            plays = bar_plays(nb, new)
            if plays.max() > 4:
                continue
            out.append(new)
            grow(new, i)

    grow([], 0)
    return out


def spans_of(nb, path):
    """The bars played, as [start, end) spans of bar indices."""
    spans, at = [], 0
    for (j, i) in path:
        spans.append((at, j))
        at = i
    spans.append((at, nb))
    return spans


def bar_plays(nb, path):
    plays = np.zeros(nb, int)
    for a, b in spans_of(nb, path):
        plays[a:b] += 1
    return plays


def drops(nb, path):
    return sum(1 for a, b in spans_of(nb, path) if a <= 48 < b)


def cost(nb, path, quality):
    """How much an arrangement risks and repeats: each seam's risk (how little its passages match)
    and a little for each seam at all; a bar heard a third time, more for a fourth; a fourth break
    and drop."""
    plays = bar_plays(nb, path)
    return (sum(quality[jp] for jp in path) + 0.15 * len(path) + 0.04 * int(np.maximum(plays - 2, 0).sum())
            + 0.1 * int(np.maximum(plays - 3, 0).sum()) + 0.5 * max(0, drops(nb, path) - 3))


def family(nb, jumps, quality):
    """The arrangements: for every length from the song's on, in steps of four bars, the one that
    risks and repeats least (`cost`)."""
    best = {}
    for path in walks(nb, jumps, max_jumps=8):
        n = sum(b - a for a, b in spans_of(nb, path))
        bucket = (n - nb) // 4
        c = cost(nb, path, quality)
        if bucket not in best or c < best[bucket][0]:
            best[bucket] = (c, path, n)
    return [best[k][1:] for k in sorted(best)]


def render(stereo, spans_s, gain):
    """The arrangement as the game plays it: each span of the song in turn, joined by a linear
    crossfade over the CROSSFADE before each join (the outgoing span's last samples fading out,
    the incoming span's samples before its start fading in)."""
    x = int(round(CROSSFADE * RATE))
    pieces = []
    for k, (s, e) in enumerate(spans_s):
        a, b = int(round(s * RATE)), min(int(round(e * RATE)), len(stereo))
        seg = stereo[a:b].copy()
        if k + 1 < len(spans_s):
            nxt = int(round(spans_s[k + 1][0] * RATE))
            ramp = (np.arange(x, dtype=np.float32) / x)[:, None]
            seg[-x:] = seg[-x:] * (1 - ramp) + stereo[nxt - x:nxt] * ramp
        pieces.append(seg)
    return np.concatenate(pieces) * (10 ** (gain / 20))


def write_wav(path, y):
    pcm = (np.clip(y, -1, 1) * 32767).astype('<i2')
    import wave
    with wave.open(path, 'wb') as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(pcm.tobytes())


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('master')
    ap.add_argument('--preview', default=None)
    ap.add_argument('--no-ogg', action='store_true')
    a = ap.parse_args()
    stereo = load(a.master)
    length = len(stereo) / RATE
    beats, bars = track(a.master)
    nb = len(bars) - 1
    print(f'{len(beats)} beats, {len(bars)} bars, {length:.2f} s')
    moments = dict(MOMENTS)
    moments['loudest'] = bars[LOUDEST_BAR]
    moments['after'] = bars[LOUDEST_BAR + 1]

    f = Features(stereo)
    found = scores(f, bars)
    rhythm = Rhythm(stereo)
    quality = {}
    seams = {}
    for (j, i) in JUMPS:
        s = found[(j, i)]
        if not passes(s):
            sys.exit(f'the jump back from bar {j} to bar {i} no longer passes the seam checks: {s}')
        so, si, lag, rl, ro = align(stereo, rhythm, bars[j], bars[i])
        if min(rl, ro) < LEAST_MATCH:
            sys.exit(f'the jump back from bar {j} to bar {i}: its passages match too little (low end {rl:.2f}, onsets {ro:.2f})')
        seams[(j, i)] = (so, si)
        # (A seam's risk: how little its passages match.)
        quality[(j, i)] = 1.0 - min(rl, ro)
        print(f'jump {j:2d} -> {i:2d} (+{j - i:2d} bars): seam {so:.4f} -> {si:.4f} (bars {bars[j]:.3f} -> {bars[i]:.3f}, aligned {lag * 1000:+.1f} ms); '
              f'match: low end {rl:.2f}, onsets {ro:.2f}; seam percentiles {s["seam"][0]:.0f}/{s["seam"][1]:.0f}/{s["seam"][2]:.0f}, '
              f'two bars after {s["after"]:.2f} before {s["before"]:.2f}')
    arrangements = []
    for path, n in family(nb, JUMPS, quality):
        spans_s = []
        at = 0.0
        for (j, i) in path:
            so, si = seams[(j, i)]
            spans_s.append((at, so))
            at = si
        spans_s.append((at, round(length, 6)))
        arrangements.append((path, n, spans_s))
    print(f'{len(arrangements)} arrangements, {arrangements[0][1]} to {arrangements[-1][1]} bars')

    # The cue sheet.
    out = []
    w = out.append
    w('# The credits\' song: "The War Reborn" by Hadal Static, (c) Hadal Static, used with permission')
    w('# in TA Reborn (assets/reborn/music/NOTICE.txt). Written by tools/credits/song.py from the band\'s')
    w('# master; the credits\' film (credits_show.rs) is timed to it. Seconds of the song.')
    w(f'path = "{GAME_PATH}"')
    w(f'length = {length:.6f}')
    w(f'gain_db = {GAIN_DB}')
    w(f'crossfade = {CROSSFADE}')
    w('# Every bar\'s first beat.')
    w('bars = [' + ', '.join(f'{b:.3f}' for b in bars) + ']')
    w('')
    w('# The moments the film lands on.')
    w('[moments]')
    for k, v in moments.items():
        w(f'{k} = {v:.3f}')
    w('')
    w('# The song made longer, shortest first: the spans of it played in turn ([from, to], seconds),')
    w('# each join a jump back between two bars where the music is as it would have been.')
    for path, n, spans_s in arrangements:
        w('[[arrangement]]')
        w(f'bars = {n}')
        w('jumps = [' + ', '.join(f'[{j}, {i}]' for j, i in path) + ']')
        w('spans = [' + ', '.join(f'[{s:.6f}, {e:.6f}]' for s, e in spans_s) + ']')
        w('')
    text = '\n'.join(out)
    with open(CUE, 'w', encoding='utf-8', newline='\n') as fh:
        fh.write(text + '\n')
    print(CUE)

    if not a.no_ogg:
        os.makedirs(os.path.dirname(OGG), exist_ok=True)
        subprocess.run(['ffmpeg', '-v', 'error', '-y', '-i', a.master, '-map', '0:a', '-ac', '2', '-ar', str(RATE),
                        '-c:a', 'libvorbis', '-q:a', '8',
                        '-metadata', 'title=The War Reborn', '-metadata', 'artist=Hadal Static',
                        '-metadata', 'copyright=(c) Hadal Static, used with permission in TA Reborn. All rights reserved.',
                        OGG], check=True)
        print(OGG, os.path.getsize(OGG), 'bytes')

    if a.preview:
        os.makedirs(a.preview, exist_ok=True)
        for path, n, spans_s in arrangements:
            y = render(stereo, spans_s, 0.0)
            name = os.path.join(a.preview, f'arrangement_{n:03d}.wav')
            write_wav(name, y)
            print(f'{name}: {len(y) / RATE:.1f} s, jumps {path}')
        json.dump({'bars': bars, 'beats': beats, 'moments': moments,
                   'arrangements': [{'bars': n, 'jumps': p, 'spans': s} for p, n, s in arrangements]},
                  open(os.path.join(a.preview, 'song.json'), 'w'), indent=1)


if __name__ == '__main__':
    main()
