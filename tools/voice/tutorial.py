"""The tutorial's spoken lines: every step's instruction (and its classic-interface words) and
every hint, in English and in each language of lang/ that the voice speaks, written as Ogg Vorbis
to assets/reborn/voice/tutorial/<language>/<id>.ogg. <id> is the start of the SHA-256 of the
line's English words with the default keys and mouse buttons filled in, as the game names them
(`tutorial::voice_path`): a line whose English words change, or one a player has rebound a key
of, has no clip and is not spoken; a language without a clip of a line falls back to English's.

The voice is "Heart" (af_heart) of Kokoro-82M (hexgrad, Apache-2.0). It says the English lines
itself (--kokoro: tools/voice/kokoro_say.py, run by the Python of a venv of its own); the other
languages are Chatterbox Multilingual (Resemble AI, MIT licence) on this computer's GPU, speaking
with Heart's timbre from tools/voice/heart.wav (Kokoro's Heart saying the first lesson's welcome).
Every clip is heard back by Whisper (large-v3-turbo, MIT) and made again (another seed) when what
Whisper hears is too far from the words. Each language's index.toml keeps the words each clip
says: a clip whose words (a translation) changed is made again, clips of lines no longer spoken
are removed. Ukrainian is not among the voice's languages: its players hear English.

The voice says a line a sentence at a time, and after a sentence's last word it often goes on:
a breath, a mumble, a click, half a syllable, a hiss, now and then words of its own. So each
sentence is cut to its words as it is made (`cut`): Whisper (large-v3, through faster-whisper)
hears where each word is, the sentence's sound is found from its level, and what comes after
the last word (and before the first) goes, but for a short tail that only dies away; a cut that
took away anything that sounds like speech is heard again, and one that lost a word makes the
take a failure (another seed). The sentences are joined with a short pause and the line levelled
to one loudness, its rare sharp peaks limited rather than the whole line turned down for them.

Setup (Python 3.13, an NVIDIA GPU, ffmpeg on the PATH), in a folder of its own:
    python -m venv venv
    venv/Scripts/python -m pip install torch==2.6.0 torchaudio==2.6.0 --index-url https://download.pytorch.org/whl/cu124
    venv/Scripts/python -m pip install chatterbox-tts==0.1.7 faster-whisper==1.2.1
and Kokoro's own venv as tools/voice/kokoro_say.py says. Then, from the repository's root:
    <venv>/Scripts/python tools/voice/tutorial.py --kokoro <kokoro venv>/Scripts/python
        [--only en,de] [--ids <id,ru:id>] [--dry-run] [--report <file.json>] [--voice <clip.wav>]
--voice gives another clip to clone (ten seconds or so, of a voice whose owner agreed to it; the
default is heart.wav, "none" the model's own); without --kokoro English is cloned as well.

--check hears every clip there is back and lists those too far from their words, and those with
sound after their last word (whatever run made them); --check --redo removes the first, and a run
with another --seed makes them again. --ids <id,ru:id> makes those clips again (in every language,
or the one named) whatever their words. --clean cuts clips made before the cut existed as new
ones are cut, and lists those it could not (make them again with --ids); with --report it carries
on where a stopped run left off.
"""
import argparse
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import tomllib

import numpy as np

ROOT = pathlib.Path(__file__).resolve().parents[2]
OUT = ROOT / "assets" / "reborn" / "voice" / "tutorial"
# The game's languages: the voice's own code for each (None: not spoken).
VOICES = {
    "en": "en", "de": "de", "es": "es", "fr": "fr", "it": "it", "ja": "ja", "ko": "ko", "pl": "pl",
    "pt-BR": "pt", "ru": "ru", "tr": "tr", "uk": None, "zh-Hans": "zh", "zh-Hant": "zh",
}
# Whisper's names of them.
WHISPER = {"en": "english", "de": "german", "es": "spanish", "fr": "french", "it": "italian", "ja": "japanese",
           "ko": "korean", "pl": "polish", "pt": "portuguese", "ru": "russian", "tr": "turkish", "zh": "chinese"}
# How far what Whisper hears may be from the words (characters wrong in a hundred).
MAX_ERROR = {"ja": 0.30, "zh": 0.25, "ko": 0.25}
DEFAULT_MAX_ERROR = 0.15
TRIES = 4
# About how many characters a second each voice speaks (a take far longer than its words
# warrant has run on: babbling or a loop, made again).
PACE = {"ja": 7.0, "zh": 4.5, "ko": 6.0}
DEFAULT_PACE = 14.0
RATE = 24_000
PAUSE = 0.28  # seconds between a line's sentences
# The cut (`cut`): the level a frame of sound needs to count as speech, under the sentence's own
# (dB): DROP, or DROP_HISS for one with most of its energy over 4 kHz (a last "s", "sh", "f").
HOP = 0.005
DROP, DROP_HISS, HISS = 30.0, 38.0, 0.35
GAP = 0.08  # a quiet this long parts bursts of sound (a stop's closure inside a word is shorter)
QUIET = 0.8  # a sentence's pauses are shorter than this (a longer quiet in one is the voice's noise)
MARGIN = 0.12  # a burst starting this soon after Whisper's end of the last word is still it...
OVER = 0.50  # ...and the line ends no later than this after it (Whisper's ends come early)
TAIL, FADE_OUT = 0.15, 0.05  # kept after the last word's sound (its own decay), faded out
LEAD, FADE_IN = 0.06, 0.03  # kept before the first word's sound, faded in
RISE = 6.0  # dB a kept tail may rise over its quietest so far: more is a new sound, held down...
FALL = 250.0  # ...and dB a second it must die away by at least (a word's own release is faster)
# The level of every line, and the ceiling its sharpest peaks are limited to (Vorbis adds a little).
LOUDNESS = -19.0
CEILING = 10 ** (-2.0 / 20)
QUALITY = "5"  # (Vorbis at 2 filled 2-12 kHz with noise: brighter, louder and peakier than made)
# The game's test of a clip's end (`tutorial.rs`): of the 150 ms after the last word, the word's
# own release in the first 30 ms, then quiet: the last 120 ms at least 35 dB under the clip's
# loudest 50 ms, its last 10 ms 50 dB under (faded out, not cut off).
END_TEST = (0.12, -35.0, -50.0)


def rust_strings(src: str, start_pattern: str) -> str:
    """The text of the Rust array that follows `start_pattern` (to its closing bracket)."""
    start = re.search(start_pattern, src).end()
    depth, i = 1, start
    while depth:
        depth += {"[": 1, "]": -1}.get(src[i], 0)
        i += 1
    return src[start:i]


def unescape(s: str) -> str:
    return re.sub(r"\\(.)", lambda m: {"n": "\n", "t": "\t"}.get(m.group(1), m.group(1)), s)


def lines() -> list[dict]:
    """Every spoken template in lesson order: {lesson, step, kind, template}."""
    src = (ROOT / "apps" / "ta_game" / "src" / "tutorial.rs").read_text(encoding="utf-8")
    table = rust_strings(src, r"pub const LESSONS[^=]*=\s*\[")
    lit = r'"((?:[^"\\]|\\.)*)"'
    pattern = r"title:\s*n\(" + lit + r"\)|Step::new\(\s*n\(" + lit + r"\)|\.hint\(n\(" + lit + r"\)\)|Classic::Text\(n\(" + lit + r"\)\)"
    out, lesson, step = [], -1, -1
    for m in re.finditer(pattern, table):
        title, instruction, hint, classic = m.groups()
        if title is not None:
            lesson, step = lesson + 1, -1
            continue
        if instruction is not None:
            step += 1
        kind, text = ("step", instruction) if instruction is not None else ("classic", classic) if classic is not None else ("hint", hint)
        out.append({"lesson": lesson, "step": step, "kind": kind, "template": unescape(text)})
    return out


def default_keys() -> dict[str, str]:
    src = (ROOT / "crates" / "ta_config" / "src" / "lib.rs").read_text(encoding="utf-8")
    table = rust_strings(src, r"pub const ACTIONS[^=]*=\s*&\[")
    return {a: k for a, _, k in re.findall(r'\(\s*"(\w+)",\s*"((?:[^"\\]|\\.)*)",\s*"([^"]*)"\s*\)', table)}


def fill(template: str, keys: dict[str, str], left: bool, words) -> str:
    """`keys::fill`: the template with its keys and mouse buttons filled in (`words` translates)."""
    def one(m):
        action = m.group(1)
        if action == "order_click":
            return words("Click" if left else "Right-click")
        if action == "order_drag":
            return words("Drag" if left else "Right-drag")
        if action == "deselect":
            return words("Esc / Right-click") if left else "Esc"
        return keys.get(action, "")
    return re.sub(r"\{(\w+)\}", one, words(template))


def clip_id(english: str) -> str:
    return hashlib.sha256(english.encode("utf-8")).hexdigest()[:12]


def speakable(text: str) -> str:
    """The words as the voice should read them: key names as words, no slashes."""
    text = re.sub(r"\bCtrl\+", "Control ", text)
    text = re.sub(r"\bCtrl\b", "Control", text)
    text = re.sub(r"\b(Alt|Shift)\+", r"\1 ", text)
    text = re.sub(r"\bF(\d{1,2})\b", r"F\1", text)
    text = text.replace(" / ", ", ").replace("·", ",")
    return re.sub(r"\s+", " ", text).strip()


def sentences(text: str) -> list[str]:
    """The line in sentences (the voice keeps its pace and tone over short ones), the very short
    joined to the next."""
    parts = [p.strip() for p in re.split(r"(?<=[.!?。！？])\s*", text) if p.strip()]
    out = []
    for p in parts:
        if out and len(out[-1]) < 24:
            out[-1] = out[-1] + (" " if re.search(r"[A-Za-z0-9]$", out[-1][-1:]) or out[-1].endswith((".", "!", "?")) else "") + p
        else:
            out.append(p)
    return out


def jobs() -> tuple[list[dict], dict[str, dict]]:
    """Every clip of every language: {lang, id, text, template}; and each language's strings."""
    keys = default_keys()
    catalogs = {}
    for code in VOICES:
        if code == "en":
            catalogs[code] = {}
            continue
        path = ROOT / "lang" / f"{code}.toml"
        catalogs[code] = tomllib.loads(path.read_text(encoding="utf-8")).get("strings", {}) if path.exists() else {}
    out = []
    seen = set()
    for line in lines():
        for left in (False, True):
            english = fill(line["template"], keys, left, lambda s: s)
            ident = clip_id(english)
            if ident in seen:
                continue
            seen.add(ident)
            for code, voice in VOICES.items():
                if voice is None:
                    continue
                strings = catalogs[code]
                if code != "en" and not strings.get(line["template"]):
                    continue  # (untranslated: the panel shows English, and English is heard)
                words = (lambda s: s) if code == "en" else (lambda s, t=strings: t.get(s) or s)
                out.append({"lang": code, "voice": voice, "id": ident, "text": fill(line["template"], keys, left, words),
                            "lesson": line["lesson"], "step": line["step"], "kind": line["kind"]})
    return out, catalogs


def normalised(text: str, voice: str) -> str:
    text = text.lower()
    if voice == "zh":
        try:
            import opencc  # (Traditional heard as Simplified, and the other way round)
            text = opencc.OpenCC("t2s").convert(text)
        except ImportError:
            pass
    return re.sub(r"[\W_]+", "", text)


def error_rate(heard: str, said: str, voice: str) -> float:
    a, b = normalised(heard, voice), normalised(said, voice)
    if not b:
        return 0.0
    prev = list(range(len(a) + 1))
    for j, cb in enumerate(b, 1):
        cur = [j] + [0] * len(a)
        for i, ca in enumerate(a, 1):
            cur[i] = min(prev[i] + 1, cur[i - 1] + 1, prev[i - 1] + (ca != cb))
        prev = cur
    return prev[-1] / len(b)


# ------------------------------------------------------------------------------------ the cut
def distances(a: str, e: str, free_start: bool = False) -> np.ndarray:
    """The edit distance from `e` to every prefix of `a` (index i: a[:i]); with `free_start`, to
    every stretch of `a` ending there."""
    m = len(e)
    ea = np.frombuffer(e.encode("utf-32-le"), dtype=np.uint32)
    ar = np.arange(m + 1, dtype=np.int64)
    prev = ar.copy()
    out = np.empty(len(a) + 1, dtype=np.int64)
    out[0] = m
    for i, ch in enumerate(a, 1):
        base = np.empty(m + 1, dtype=np.int64)
        base[0] = 0 if free_start else i
        base[1:] = np.minimum(prev[1:] + 1, prev[:-1] + (ea != ord(ch)))
        prev = np.minimum.accumulate(base - ar) + ar  # (a deletion carried along the row)
        out[i] = prev[-1]
    return out


def align(ws: list[dict], said: str, voice: str) -> tuple[int, int, int]:
    """The run of Whisper's words that says `said`: (first, last) and its edit distance from it;
    words heard past either end of the line (a mumble after it, say) are left out of it."""
    norm = [normalised(w["word"], voice) for w in ws]
    e = normalised(said, voice)
    ends = distances("".join(norm), e)[np.cumsum([len(s) for s in norm])]
    # Ties: more words (never a last word heard a little wrong left out), but not across a quiet
    # longer than a sentence's pauses (what follows one is the voice going on).
    last = int(np.argmin(ends))
    for k in range(last + 1, len(ws)):
        if ws[k]["start"] - ws[k - 1]["end"] > QUIET:
            break
        if ends[k] == ends[last]:
            last = k
    back = [s[::-1] for s in norm[:last + 1][::-1]]
    starts = distances("".join(back), e[::-1])[np.cumsum([len(s) for s in back])]
    k = int(len(starts) - 1 - np.argmin(starts[::-1]))
    first = last - k
    while last > first and not norm[last]:
        last -= 1
    while first < last and not norm[first]:
        first += 1
    return first, last, int(starts[k])


def reaches(ws: list[dict], first: int, last: int, said: str, voice: str) -> tuple[bool, bool]:
    """Whether the words `align` found reach the line's first word and its last (a line whose
    first or last word Whisper did not hear is not cut at that end)."""
    heard = "".join(normalised(w["word"], voice) for w in ws[first:last + 1])
    e = normalised(said, voice)
    toks = [t for t in (normalised(t, voice) for t in said.split()) if t]
    if len(toks) < 2 or voice in ("ja", "zh"):
        # (no spaces between words: a few characters stand for one, or a unit's name in Latin
        # letters for itself, Whisper hearing "Peewee" as "PV")
        n = max(1, min(3, len(e) // 4))
        latin_head, latin_tail = re.match(r"[a-z0-9]*", e).group(0), re.search(r"[a-z0-9]*$", e).group(0)
        head = latin_head if len(latin_head) > n else e[:n]
        tail = latin_tail if len(latin_tail) > n else e[-n:]
    else:
        head, tail = toks[0], toks[-1]
    d = distances(heard, e)[-1]
    # (a word missing: leaving it out of the line costs nothing, or less)
    without_head = distances(heard, e[len(head):])[-1]
    without_tail = distances(heard, e[:len(e) - len(tail)])[-1]
    return bool(d - without_head < max(1, len(head) - 1)), bool(d - without_tail < max(1, len(tail) - 1))


def frames(x: np.ndarray, rate: int) -> tuple[np.ndarray, np.ndarray]:
    """Each HOP's level (dBFS, a 25 ms window centred on it) and the share of its energy over 4 kHz."""
    w, h = int(0.025 * rate), int(HOP * rate)
    n = max(1, len(x) // h)
    pad = np.pad(x.astype(np.float64), (w // 2, w // 2 + w))
    f = pad[np.arange(n)[:, None] * h + np.arange(w)[None, :]]
    db = 20 * np.log10(np.sqrt((f ** 2).mean(axis=1)) + 1e-12)
    spec = np.abs(np.fft.rfft(f * np.hanning(w), n=1024, axis=1)) ** 2 + 1e-14
    hz = np.fft.rfftfreq(1024, 1 / rate)
    return db, spec[:, hz > 4000].sum(axis=1) / spec[:, hz > 80].sum(axis=1)


def bursts(speech: np.ndarray, gap: int) -> list[tuple[int, int]]:
    """Runs of speech frames, quiets shorter than `gap` frames bridged: [(on, off)]."""
    idx = np.flatnonzero(speech)
    if not len(idx):
        return []
    breaks = np.flatnonzero(np.diff(idx) > gap)
    return list(zip(np.concatenate([[idx[0]], idx[breaks + 1]]).tolist(), np.concatenate([idx[breaks] + 1, [idx[-1] + 1]]).tolist()))


def bounds(x: np.ndarray, rate: int, ws: list[dict], first: int, last: int) -> tuple[float, float, np.ndarray, float]:
    """Where a sentence's words start and end in `x` (seconds), which frames sound like speech,
    and the level under which nothing does (dBFS). Its sound in bursts (parted by quiets of GAP):
    it ends with the last burst that begins before Whisper's end of its last word (MARGIN later at
    most), no later than OVER after it, and starts with the first burst that ends after Whisper's
    start of its first word."""
    db, high = frames(x, rate)
    n = len(db)
    w0, w1 = ws[first], ws[last]
    span = db[min(int(w0["start"] / HOP), n - 1):max(int(w1["end"] / HOP) + 1, 1)]
    level = float(np.percentile(span, 95)) if len(span) else float(db.max())
    speech = (db > level - DROP) | ((db > level - DROP_HISS) & (high > HISS))
    bs = bursts(speech, int(round(GAP / HOP)))
    # (words of its own the voice went on with: the line ends before them)
    later = [w["start"] for w in ws[last + 1:] if w["start"] >= w1["end"]]
    cap = min([w1["end"] + OVER] + [s + 0.05 for s in later[:1]])
    ends = [off * HOP for on, off in bs if on * HOP <= w1["end"] + MARGIN and off * HOP >= w0["start"]]
    starts = [on * HOP for on, off in bs if off * HOP >= w0["start"] - MARGIN and on * HOP <= w1["end"]]
    end = min(max(ends), cap) if ends else w1["end"]
    start = max(min(starts), w0["start"] - OVER) if starts else w0["start"]
    return max(0.0, start), min(end, len(x) / rate), speech, level - DROP


def hold_down(x: np.ndarray, rate: int, quiet: float = 0.0) -> np.ndarray:
    """A tail allowed only to die away from `quiet` (dBFS) at most: wherever it rises more than
    RISE dB over the quietest it has been (a click, a breath, a hum starting), or falls slower than
    FALL (a hum or a hiss going on), it is turned down to that. (A tail cut in the middle of the
    voice's noise, the noise going on right from the last word, starts no louder than `quiet`.)"""
    w, h = int(0.01 * rate), int(0.0025 * rate)
    if len(x) <= w:
        return x
    pad = np.pad(x.astype(np.float64), (w // 2, w), mode="reflect")
    n = len(x) // h + 1
    db = 20 * np.log10(np.sqrt((pad[np.arange(n)[:, None] * h + np.arange(w)[None, :]] ** 2).mean(axis=1)) + 1e-9)
    ceiling = np.minimum(np.minimum.accumulate(db) + RISE, min(db[0], quiet) - FALL * np.arange(n) * h / rate)
    over = np.maximum(db - ceiling, 0.0)
    over = np.convolve(np.pad(over, 2, mode="edge"), np.ones(5) / 5, mode="valid")  # (no clicks of its own)
    return (x * np.interp(np.arange(len(x)), np.arange(n) * h, 10 ** (-over / 20))).astype(np.float32)


def fade(n: int) -> np.ndarray:
    return (0.5 - 0.5 * np.cos(np.linspace(0, np.pi, n))).astype(np.float32)


def trim(x: np.ndarray, rate: int, start: float, end: float, cut_start: bool = True, cut_end: bool = True,
         quiet: float = 0.0) -> np.ndarray:
    """`x` from LEAD before `start` to TAIL after `end` (seconds): the tail held down to its
    decay from `quiet` (dBFS) at most, made up with silence where the voice stopped short, and
    faded out; the lead-in held down the same way and faded in."""
    a = max(0, int(round((start - LEAD) * rate))) if cut_start else 0
    s = int(round(start * rate)) if cut_start else 0
    e = min(len(x), int(round(end * rate))) if cut_end else len(x)
    b = min(len(x), int(round((end + TAIL) * rate))) if cut_end else len(x)
    head = hold_down(x[a:s][::-1], rate, quiet)[::-1] if cut_start else x[a:s]
    tail = hold_down(x[e:b], rate, quiet) if cut_end else x[e:b]
    if cut_end:
        tail = np.concatenate([tail, np.zeros(max(0, int(round(TAIL * rate)) - len(tail)), dtype=np.float32)])
    out = np.concatenate([head, x[s:e], tail]).astype(np.float32)
    n_in, n_out = min(int(FADE_IN * rate), len(head)), min(int(FADE_OUT * rate), len(out) // 2)
    if cut_start and n_in > 1:
        out[:n_in] *= fade(n_in)
    if cut_end and n_out > 1:
        out[-n_out:] *= fade(n_out)[::-1]
    return out


_words_model = None


def hear_words(x: np.ndarray, rate: int, voice: str, words: bool = True) -> tuple[str, list[dict]]:
    """What Whisper (large-v3) hears in `x`, and where each word is (seconds)."""
    global _words_model
    if _words_model is None:
        import torch
        if hasattr(os, "add_dll_directory"):  # (CTranslate2 finds cuBLAS among PyTorch's libraries)
            os.add_dll_directory(str(pathlib.Path(torch.__file__).parent / "lib"))
        from faster_whisper import WhisperModel
        _words_model = WhisperModel("large-v3", device="cuda", compute_type="float16")
    import librosa
    audio = librosa.resample(x.astype(np.float32), orig_sr=rate, target_sr=16_000)
    segments, _ = _words_model.transcribe(audio, language=voice, word_timestamps=words, beam_size=5,
                                          condition_on_previous_text=False, vad_filter=False)
    text, ws = "", []
    for s in segments:
        text += s.text
        ws += [{"word": w.word, "start": w.start, "end": w.end} for w in (s.words or [])]
    return text, ws


def cut(x: np.ndarray, rate: int, said: str, voice: str) -> tuple[np.ndarray, str | None, dict]:
    """A sentence as the voice said it, cut to its words (`bounds`, `trim`) and heard again: and
    why it is not to be trusted (None when it is): no words heard, its last word not heard (its
    end left uncut), or the cut sentence further from its words than the whole was (junk aside),
    a word lost. A first word Whisper does not seem to hear (often a unit's English name heard in
    the language's own script, "Kbot" as "Keyboard") leaves the start uncut: the voice seldom
    makes noise before a sentence. And what was cut away: its loudest 25 ms after the kept tail
    and before the lead-in (dBFS), how long, the words heard there."""
    text, ws = hear_words(x, rate, voice)
    if not ws:
        return x, "no words heard", {}
    first, last, _ = align(ws, said, voice)
    has_start, has_end = reaches(ws, first, last, said, voice)
    start, end, speech, quiet = bounds(x, rate, ws, first, last)
    if not has_start:
        start = 0.0  # (left as it was: nothing before it cut, so nothing of it lost)
    out = trim(x, rate, start, end, has_start, has_end, quiet)
    db, _ = frames(x, rate)
    t = np.arange(len(db)) * HOP
    after, before = db[t >= end + TAIL], db[t < start - LEAD]
    info = {"seconds": round(len(x) / rate, 3), "start": round(start, 3), "end": round(end, 3),
            "after_s": round(max(0.0, len(x) / rate - end - TAIL), 3), "after_db": round(float(after.max()), 1) if len(after) else None,
            "before_s": round(max(0.0, start - LEAD), 3), "before_db": round(float(before.max()), 1) if len(before) else None,
            "words_after": "".join(w["word"] for w in ws[last + 1:]).strip()}
    if not has_end:
        return out, "its last word not heard", info
    inside = [(on, off) for on, off in bursts(speech, int(round(GAP / HOP))) if off * HOP > start and on * HOP < end]
    longest = max([(b[0] - a[1]) * HOP for a, b in zip(inside, inside[1:])], default=0.0)
    if longest > QUIET:
        return out, f"a quiet of {longest:.1f} s inside it (the voice's noise, words of its own after it)", info
    if not speech[(t < start) | (t > end)].any():
        return out, None, info  # (nothing that sounds like speech cut or held down: its words are as they were)
    # Heard again: its last word still there, and no further from its words than the whole was,
    # junk aside (give or take Whisper hearing an odd name differently from one hearing to the next).
    e = normalised(said, voice)
    whole = int(distances(normalised(text, voice), e, free_start=True).min())
    text_cut, ws_cut = hear_words(out, rate, voice)
    after_cut = int(distances(normalised(text_cut, voice), e)[-1])
    ends = bool(ws_cut) and reaches(ws_cut, *align(ws_cut, said, voice)[:2], said, voice)[1]
    if not ends or after_cut > whole + 2 + 0.05 * len(e):
        return out, f"cut further from its words ({after_cut} wrong, {whole} before{'' if ends else ', its last word gone'})", info
    return out, None, info


def parts(x: np.ndarray, rate: int) -> list[tuple[int, int]]:
    """A line's sentences in a clip: the samples between its pauses (runs of silence)."""
    small = np.abs(x) < 2e-4
    edges = np.flatnonzero(np.diff(np.concatenate([[0], small.astype(np.int8), [0]])))
    out, start = [], 0
    for a, b in zip(edges[::2], edges[1::2]):
        if (b - a) / rate >= 0.2:
            out.append((start, int(a)))
            start = int(b)
    return out + [(start, len(x))]


def joined(pieces: list[np.ndarray], rate: int) -> np.ndarray:
    gap = np.zeros(int(PAUSE * rate), dtype=np.float32)
    return np.concatenate([p for k, piece in enumerate(pieces) for p in ([gap, piece] if k else [piece])])


def limit(x: np.ndarray, rate: int, ceiling: float = CEILING) -> np.ndarray:
    """A look-ahead peak limiter: the gain eases down over 5 ms before a peak over `ceiling` and
    back up over 80 ms after it."""
    x = x.astype(np.float64)
    blk = max(1, int(rate * 0.001))
    nb = -(-len(x) // blk)
    need = np.minimum(1.0, ceiling / np.maximum(np.pad(np.abs(x), (0, nb * blk - len(x))).reshape(nb, blk).max(axis=1), 1e-12))
    if need.min() >= 1.0:
        return x.astype(np.float32)
    g = need.copy()
    for j in range(1, 6):
        g = np.minimum(g, 1 - (1 - np.concatenate([need[j:], np.ones(j)])) * (1 - j / 6))
    a, cur, out = 1 - np.exp(-1 / 80), 1.0, np.empty_like(g)
    for k in range(nb):
        cur = min(g[k], cur + (1 - cur) * a)
        out[k] = cur
    gain = np.convolve(np.pad(np.repeat(out, blk)[:len(x)], blk, mode="edge"), np.ones(2 * blk + 1) / (2 * blk + 1), mode="valid")
    return (x * np.minimum(gain, ceiling / np.maximum(np.abs(x), ceiling))).astype(np.float32)


def levelled(audio: np.ndarray, rate: int = RATE) -> np.ndarray:
    """At LOUDNESS (integrated, LUFS), its peaks over CEILING limited (turning a whole line down
    for one sharp "p" left some lines quieter than the rest)."""
    import pyloudnorm
    meter = pyloudnorm.Meter(rate)
    x = audio.astype(np.float64)
    gain = LOUDNESS - meter.integrated_loudness(x)
    for _ in range(2):  # (limiting takes a little off: made up)
        gain += LOUDNESS - meter.integrated_loudness(limit(x * 10 ** (gain / 20), rate).astype(np.float64))
    return limit(x * 10 ** (gain / 20), rate)


def end_test(x: np.ndarray, rate: int) -> tuple[float, float]:
    """The game's test of a clip's end: its last 120 ms's loudest 10 ms and its last 10 ms, in dB
    under its loudest 50 ms."""
    def levels(y, win, hop):
        w, h = int(win * rate), int(hop * rate)
        idx = np.arange(0, max(len(y) - w, 0) + 1, h)[:, None] + np.arange(w)[None, :]
        return 20 * np.log10(np.sqrt((y[np.minimum(idx, len(y) - 1)].astype(np.float64) ** 2).mean(axis=1)) + 1e-9)
    loud = levels(x, 0.05, 0.01).max()
    return float(levels(x[-int(END_TEST[0] * rate):], 0.01, 0.01).max() - loud), float(levels(x[-int(0.01 * rate):], 0.01, 0.01).max() - loud)


def read(path: pathlib.Path) -> tuple[np.ndarray, int]:
    import soundfile as sf
    x, rate = sf.read(path, dtype="float32", always_2d=False)
    return (x.mean(axis=1) if x.ndim > 1 else x).astype(np.float32), rate


def write(audio: np.ndarray, rate: int, path: pathlib.Path) -> None:
    """Ogg Vorbis, mono, nothing but the sound in it (the same bytes from the same sound)."""
    import soundfile as sf
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        wav = pathlib.Path(tmp) / "line.wav"
        sf.write(wav, audio, rate, subtype="PCM_16")
        subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-i", str(wav), "-ac", "1", "-c:a", "libvorbis", "-q:a", QUALITY,
                        "-map_metadata", "-1", "-fflags", "+bitexact", "-flags:a", "+bitexact", str(path)], check=True)


# ------------------------------------------------------------------------------------ the runs
def turbo():
    import torch
    from transformers import pipeline
    return pipeline("automatic-speech-recognition", model="openai/whisper-large-v3-turbo", torch_dtype=torch.float16, device="cuda")


def heard_by(asr, audio: np.ndarray, rate: int, voice: str) -> str:
    import librosa
    audio = librosa.resample(audio.astype(np.float32), orig_sr=rate, target_sr=16_000) if rate != 16_000 else audio
    return asr({"raw": audio, "sampling_rate": 16_000}, return_timestamps=len(audio) / 16_000 > 28,
               generate_kwargs={"language": WHISPER[voice], "task": "transcribe"})["text"]


def check(work: list[dict], redo: bool, report: str | None) -> None:
    """Every clip there is of `work`'s lines, heard back by Whisper: those further from their words
    than their language allows listed (and with `redo` removed), and those with sound in their
    end (the game's test, `end_test`)."""
    asr = turbo()
    rows, far, loud = [], [], []
    for j in work:
        ogg = OUT / j["lang"] / f"{j['id']}.ogg"
        if not ogg.exists():
            continue
        audio, rate = read(ogg)
        heard = heard_by(asr, audio, rate, j["voice"])
        err = error_rate(heard, speakable(j["text"]), j["voice"])
        tail, last = end_test(audio, rate)
        rows.append({**j, "error": round(err, 3), "heard": heard, "seconds": round(len(audio) / rate, 2), "tail": round(tail, 1), "last": round(last, 1)})
        if err > MAX_ERROR.get(j["voice"], DEFAULT_MAX_ERROR):
            far.append((j, err, heard))
        if tail > END_TEST[1] or last > END_TEST[2]:
            loud.append((j, tail, last))
    for j, err, heard in far:
        print(f"{j['lang']} {j['id']} err {err:.2f}\n    said:  {j['text']}\n    heard: {heard.strip()}")
        if redo:
            (OUT / j["lang"] / f"{j['id']}.ogg").unlink()
    for j, tail, last in loud:
        print(f"{j['lang']} {j['id']}: sound at its end (last 120 ms {tail:.0f} dB, last 10 ms {last:.0f} dB under its loudest)")
    print(f"{len(rows)} clips heard, {len(far)} too far from their words" + (" (removed)" if redo and far else "")
          + f", {len(loud)} with sound at their end")
    if report:
        pathlib.Path(report).write_text(json.dumps(rows, ensure_ascii=False, indent=1), encoding="utf-8")


def clean(work: list[dict], report: str | None) -> None:
    """Clips made before the cut, cut as new ones are: each sentence (between the pauses) cut to
    its words, joined and levelled again. A clip with a sentence whose cut is not to be trusted is
    left as it was and listed (--check then hears the clips whole). With a report, each clip is
    added to it as it is done, and a run stopped carries on where it was."""
    rows = json.loads(pathlib.Path(report).read_text(encoding="utf-8")) if report and pathlib.Path(report).exists() else []
    done = {(r["lang"], r["id"]) for r in rows}
    for n, j in enumerate(work, 1):
        ogg = OUT / j["lang"] / f"{j['id']}.ogg"
        if not ogg.exists() or (j["lang"], j["id"]) in done:
            continue
        audio, rate = read(ogg)
        said = sentences(speakable(j["text"]))
        spans = parts(audio, rate)
        notes = [] if len(spans) == len(said) else [f"{len(spans)} parts for {len(said)} sentences"]
        pieces, cuts = [], []
        for k, (a, b) in enumerate(spans if not notes else []):
            piece, note, info = cut(audio[a:b], rate, said[k], j["voice"])
            pieces.append(piece)
            cuts.append(info)
            if note:
                notes.append(f"sentence {k + 1}: {note}")
        tail, last = end_test(audio, rate)
        row = {"lang": j["lang"], "id": j["id"], "before": round(len(audio) / rate, 2), "sentences": cuts,
               "end_before": [round(tail, 1), round(last, 1)]}
        if not notes:
            new = levelled(joined(pieces, rate), rate)
            tail, last = end_test(new, rate)
            if tail > END_TEST[1] or last > END_TEST[2]:
                notes.append(f"its end still not quiet ({tail:.0f} dB)")
            else:
                row.update({"after": round(len(new) / rate, 2), "end_after": [round(tail, 1), round(last, 1)]})
                write(new, rate, ogg)
        row["notes"] = notes
        rows.append(row)
        if report:
            pathlib.Path(report).write_text(json.dumps(rows, ensure_ascii=False, indent=1), encoding="utf-8")
        print(f"[{n}/{len(work)}] {j['lang']} {j['id']} {row['before']:5.2f}s -> {row.get('after', row['before']):5.2f}s"
              + (f"  KEPT: {'; '.join(notes)}" if notes else ""), flush=True)
    kept = [r for r in rows if r["notes"]]
    print(f"{len(rows)} clips: {len(rows) - len(kept)} cut, {len(kept)} left as they were"
          + (f" (make them again: --ids {','.join(sorted(r['lang'] + ':' + r['id'] for r in kept))})" if kept else ""))


def main():
    # (Its reports print every language's words: never through Windows' code page.)
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--only", help="languages, comma-separated (en, de, zh-Hant...)")
    ap.add_argument("--ids", help="only these clips, made again whatever their words (comma-separated ids, or language:id)")
    ap.add_argument("--dry-run", action="store_true", help="list what would be made")
    ap.add_argument("--report", help="write every clip's check (what Whisper heard) to this JSON file")
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--limit", type=int, help="make only the first n lines of each language (trials)")
    ap.add_argument("--out", help="write here instead of assets/reborn/voice/tutorial (trials)")
    ap.add_argument("--voice", default=str(pathlib.Path(__file__).with_name("heart.wav")),
                    help="a clip of the voice to clone (default heart.wav; none: the model's own)")
    ap.add_argument("--kokoro", help="the Python of a venv with Kokoro: English said by Kokoro's own voice (kokoro_say.py)")
    ap.add_argument("--kokoro-voice", default="af_heart", help="Kokoro's voice for English (af_heart: Heart)")
    ap.add_argument("--check", action="store_true", help="hear every clip made back and list those too far from their words")
    ap.add_argument("--redo", action="store_true", help="with --check: remove those, so the next run makes them again (give it another --seed)")
    ap.add_argument("--clean", action="store_true", help="cut clips made before the cut as new ones are")
    args = ap.parse_args()
    global OUT
    if args.out:
        OUT = pathlib.Path(args.out)
    only = set(args.only.split(",")) if args.only else None
    ids = set(args.ids.split(",")) if args.ids else None
    work, _ = jobs()
    if only:
        work = [j for j in work if j["lang"] in only]
    if args.limit:
        work = [j for k, j in enumerate(work) if sum(1 for x in work[:k] if x["lang"] == j["lang"]) < args.limit]
    all_work = work
    chosen = lambda j: j["id"] in ids or f"{j['lang']}:{j['id']}" in ids
    if ids:
        work = [j for j in work if chosen(j)]
    # What is there already: kept when it says the same words (every language's index read, as
    # each is written again at the end).
    todo, keep = [], {}
    for j in all_work:
        index = OUT / j["lang"] / "index.toml"
        said = keep.setdefault(j["lang"], tomllib.loads(index.read_text(encoding="utf-8")).get("lines", {}) if index.exists() else {})
        if ids:
            if chosen(j):
                todo.append(j)
        elif said.get(j["id"]) != j["text"] or not (OUT / j["lang"] / f"{j['id']}.ogg").exists():
            todo.append(j)
    todo.sort(key=lambda j: j["lang"] != "en")
    by_lang = {}
    for j in all_work:
        by_lang.setdefault(j["lang"], []).append(j)
    for code, js in by_lang.items():
        print(f"{code}: {len(js)} lines, {sum(len(j['text']) for j in js)} characters, {sum(1 for j in todo if j['lang'] == code)} to make")
    if args.dry_run:
        return
    if args.check:
        return check(work, args.redo, args.report)
    if args.clean:
        return clean(work, args.report)

    import subprocess
    import tempfile
    import time

    import torch
    # English by Kokoro's own voice: its server, talked to a line at a time.
    kokoro = None
    if args.kokoro and any(j["voice"] == "en" for j in todo):
        kokoro = subprocess.Popen([args.kokoro, str(pathlib.Path(__file__).with_name("kokoro_say.py")), args.kokoro_voice],
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, encoding="utf-8")
        # (Kokoro's libraries may say things first.)
        for said in kokoro.stdout:
            if said.strip() == "ready":
                break
        else:
            sys.exit("kokoro_say.py did not start (see its setup)")
    scratch = pathlib.Path(tempfile.mkdtemp(prefix="tutorial_voice_"))
    tts = None
    cap = [1000]
    if any(j["voice"] != "en" or kokoro is None for j in todo):
        from chatterbox.mtl_tts import ChatterboxMultilingualTTS
        # (TensorFloat-32 matrix products: much faster on recent GPUs, no difference to hear.)
        torch.backends.cuda.matmul.allow_tf32 = True
        torch.backends.cudnn.allow_tf32 = True
        tts = ChatterboxMultilingualTTS.from_pretrained(device="cuda")
        # The model may run on (a loop, babbling) for up to 1000 speech tokens, 40 seconds: each
        # sentence is held to about twice the time its words take (25 tokens a second).
        inference = tts.t3.inference

        def capped(*a, **k):
            k["max_new_tokens"] = min(k.get("max_new_tokens") or 1000, cap[0])
            return inference(*a, **k)
        tts.t3.inference = capped
        if args.voice and args.voice.lower() != "none":
            tts.prepare_conditionals(args.voice, exaggeration=0.45)
    asr = turbo()
    report = []

    def kokoro_says(text: str) -> tuple[np.ndarray, list[str]]:
        """The line by Kokoro, as one passage (it paces its sentences itself), cut to its words and
        levelled as Chatterbox's are."""
        path = scratch / "line.wav"
        kokoro.stdin.write(json.dumps({"text": speakable(text), "out": str(path)}) + "\n")
        kokoro.stdin.flush()
        answer = kokoro.stdout.readline().strip()
        if answer != "ok":
            return np.zeros(RATE // 10, np.float32), [f"kokoro: {answer or 'no answer'}"]
        audio, rate = read(path)
        if rate != RATE:
            import librosa
            audio = librosa.resample(audio, orig_sr=rate, target_sr=RATE)
        piece, note, _ = cut(audio, RATE, speakable(text), "en")
        notes = [note] if note else []
        line = levelled(piece)
        tail, last = end_test(line, RATE)
        if tail > END_TEST[1] or last > END_TEST[2]:
            notes.append(f"its end not quiet ({tail:.0f} dB)")
        return line, notes

    def speak(text: str, voice: str, seed: int) -> tuple[np.ndarray, list[str]]:
        """The line said a sentence at a time, each cut to its words: and what in it is not to be trusted."""
        if voice == "en" and kokoro is not None:
            return kokoro_says(text)
        pieces, notes = [], []
        for k, sentence in enumerate(sentences(speakable(text))):
            torch.manual_seed(seed * 1000 + k)
            cap[0] = int(max(4.0, 2.0 * len(sentence) / PACE.get(voice, DEFAULT_PACE)) * 25) + 40
            # (The default voice is English's: with no guidance from it, other languages keep
            # their own accent.)
            wav = tts.generate(sentence, language_id=voice, exaggeration=0.45, cfg_weight=0.5 if voice == "en" else 0.0, temperature=0.75)
            audio = wav.squeeze(0).detach().cpu().numpy()
            if tts.sr != RATE:
                import librosa
                audio = librosa.resample(audio, orig_sr=tts.sr, target_sr=RATE)
            piece, note, _ = cut(audio, RATE, sentence, voice)
            pieces.append(piece)
            if note:
                notes.append(f"sentence {k + 1}: {note}")
        line = levelled(joined(pieces, RATE))
        tail, last = end_test(line, RATE)
        if tail > END_TEST[1] or last > END_TEST[2]:
            notes.append(f"its end not quiet ({tail:.0f} dB)")
        return line, notes

    started = time.time()
    for n, j in enumerate(todo, 1):
        best = None
        limit_err = MAX_ERROR.get(j["voice"], DEFAULT_MAX_ERROR)
        for attempt in range(1 if j["voice"] == "en" and kokoro is not None else TRIES):
            audio, notes = speak(j["text"], j["voice"], args.seed + attempt)
            seconds = len(audio) / RATE
            expected = len(speakable(j["text"])) / PACE.get(j["voice"], DEFAULT_PACE)
            if seconds > max(8.0, 2.2 * expected):
                heard, err = f"(ran on: {seconds:.0f} s for about {expected:.0f})", 1.0
            else:
                heard = heard_by(asr, audio, RATE, j["voice"])
                err = error_rate(heard, speakable(j["text"]), j["voice"])
            # (a take with a sentence whose cut is not to be trusted is never good enough: next seed)
            score = err + (1.0 if notes else 0.0)
            if best is None or score < best[0]:
                best = (score, err, audio, heard, attempt, notes)
            if score <= limit_err:
                break
        _, err, audio, heard, attempt, notes = best
        folder = OUT / j["lang"]
        write(audio, RATE, folder / f"{j['id']}.ogg")
        keep[j["lang"]][j["id"]] = j["text"]
        write_index(folder, keep[j["lang"]], {x["id"] for x in all_work if x["lang"] == j["lang"]})
        flag = "" if err <= limit_err and not notes else "  << CHECK" + (f" ({'; '.join(notes)})" if notes else "")
        left = (time.time() - started) / n * (len(todo) - n) / 60
        print(f"[{n}/{len(todo)}] {j['lang']} {j['id']} {len(audio) / RATE:4.1f}s err {err:.2f} (try {attempt + 1}), {left:.0f} min to go{flag}", flush=True)
        report.append({**j, "error": round(err, 3), "heard": heard, "seconds": round(len(audio) / RATE, 2), "try": attempt + 1, "notes": notes})
    # Lines no longer spoken: their clips go.
    for code in by_lang:
        folder = OUT / code
        wanted = {j["id"] for j in by_lang[code]}
        for ogg in folder.glob("*.ogg"):
            if ogg.stem not in wanted:
                ogg.unlink()
        if folder.exists():
            write_index(folder, keep.get(code, {}), wanted)
    if args.report:
        pathlib.Path(args.report).write_text(json.dumps(report, ensure_ascii=False, indent=1), encoding="utf-8")


def write_index(folder: pathlib.Path, said: dict, wanted: set):
    rows = [f'"{k}" = {json.dumps(v, ensure_ascii=False)}' for k, v in sorted(said.items()) if k in wanted]
    text = "# The words each clip says (tools/voice/tutorial.py): a clip is made again when they change.\n[lines]\n" + "\n".join(rows) + "\n"
    (folder / "index.toml").write_text(text, encoding="utf-8", newline="\n")


if __name__ == "__main__":
    sys.exit(main())
