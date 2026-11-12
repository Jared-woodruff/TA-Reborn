"""Writes the interface's bundled Sofia Sans (`crates/ta_ui/fonts/`) from Google Fonts' files.

Sofia Sans draws Cyrillic in its Bulgarian forms by default (в, д, и, т like Latin b, g, u, m)
and keeps the forms Russian and Ukrainian readers know behind the `locl` feature of its Russian
language system. egui shapes text with no language, so the interface would show the Bulgarian
forms. This makes the Russian forms the default: each character the Russian `locl` changes is
mapped straight to the glyph it changes to, in the character map. Nothing else in the font
changes (its outlines, metrics, kerning and other features stay as they are). The SIL Open Font
License allows such a change (Sofia Sans reserves no font name); `crates/ta_ui/fonts/README.md`
says what was changed.

    python tools/fonts/sofia_sans.py <folder> crates/ta_ui/fonts

`<folder>` holds `SofiaSansSemiCondensed[wght].ttf` and `SofiaSansExtraCondensed[wght].ttf` from
github.com/google/fonts (`ofl/sofiasanssemicondensed`, `ofl/sofiasansextracondensed`). Plain
Python, no packages.
"""

import struct
import sys
from pathlib import Path

FONTS = {
    "SofiaSansSemiCondensed[wght].ttf": "SofiaSansSemiCondensed.ttf",
    "SofiaSansExtraCondensed[wght].ttf": "SofiaSansExtraCondensed.ttf",
}


class Font:
    def __init__(self, data: bytes):
        self.data = data
        count = self.u16(4)
        self.order = []
        self.tables = {}
        for i in range(count):
            record = 12 + 16 * i
            tag = data[record:record + 4]
            offset, length = self.u32(record + 8), self.u32(record + 12)
            self.order.append(tag)
            self.tables[tag] = data[offset:offset + length]

    def u16(self, at: int) -> int:
        return struct.unpack(">H", self.data[at:at + 2])[0]

    def u32(self, at: int) -> int:
        return struct.unpack(">I", self.data[at:at + 4])[0]

    def write(self) -> bytes:
        """The font again, its tables in their order, each on a four-byte boundary, the checksums
        and head's checksum adjustment made again."""
        count = len(self.order)
        power = 1
        while power * 2 <= count:
            power *= 2
        search_range = power * 16
        header = self.data[:4] + struct.pack(">HHHH", count, search_range, power.bit_length() - 1, count * 16 - search_range)
        head = bytearray(self.tables[b"head"])
        head[8:12] = b"\0\0\0\0"
        self.tables[b"head"] = bytes(head)
        offset = 12 + 16 * count
        directory, body = b"", b""
        # (The directory sorted by tag, as the specification asks; the tables in their order.)
        placed = {}
        for tag in self.order:
            table = self.tables[tag]
            placed[tag] = (checksum(table), offset + len(body), len(table))
            body += table + b"\0" * (-len(table) % 4)
        for tag in sorted(self.order):
            directory += tag + struct.pack(">III", *placed[tag])
        font = bytearray(header + directory + body)
        adjustment = (0xB1B0AFBA - checksum(bytes(font))) & 0xFFFFFFFF
        head_at = placed[b"head"][1]
        font[head_at + 8:head_at + 12] = struct.pack(">I", adjustment)
        return bytes(font)


def checksum(table: bytes) -> int:
    table += b"\0" * (-len(table) % 4)
    return sum(struct.unpack(f">{len(table) // 4}I", table)) & 0xFFFFFFFF


def u16(data: bytes, at: int) -> int:
    return struct.unpack(">H", data[at:at + 2])[0]


def u32(data: bytes, at: int) -> int:
    return struct.unpack(">I", data[at:at + 4])[0]


def coverage(t: bytes, at: int) -> list:
    if u16(t, at) == 1:
        return [u16(t, at + 4 + 2 * i) for i in range(u16(t, at + 2))]
    glyphs = []
    for i in range(u16(t, at + 2)):
        glyphs.extend(range(u16(t, at + 4 + 6 * i), u16(t, at + 6 + 6 * i) + 1))
    return glyphs


def single_substitution(t: bytes, at: int) -> dict:
    glyphs = coverage(t, at + u16(t, at + 2))
    if u16(t, at) == 1:
        delta = struct.unpack(">h", t[at + 4:at + 6])[0]
        return {g: (g + delta) & 0xFFFF for g in glyphs}
    return {g: u16(t, at + 6 + 2 * i) for i, g in enumerate(glyphs)}


def russian_forms(gsub: bytes) -> dict:
    """What the `locl` feature of Cyrillic's Russian language system changes: glyph to glyph."""
    scripts, features, lookups = u16(gsub, 4), u16(gsub, 6), u16(gsub, 8)
    for i in range(u16(gsub, scripts)):
        if gsub[scripts + 2 + 6 * i:scripts + 6 + 6 * i] != b"cyrl":
            continue
        script = scripts + u16(gsub, scripts + 6 + 6 * i)
        for j in range(u16(gsub, script + 2)):
            if gsub[script + 4 + 6 * j:script + 8 + 6 * j] != b"RUS ":
                continue
            langsys = script + u16(gsub, script + 8 + 6 * j)
            mapping = {}
            for k in range(u16(gsub, langsys + 4)):
                feature = u16(gsub, langsys + 6 + 2 * k)
                if gsub[features + 2 + 6 * feature:features + 6 + 6 * feature] != b"locl":
                    continue
                table = features + u16(gsub, features + 6 + 6 * feature)
                for m in range(u16(gsub, table + 2)):
                    lookup = lookups + u16(gsub, lookups + 2 + 2 * u16(gsub, table + 4 + 2 * m))
                    kind = u16(gsub, lookup)
                    for n in range(u16(gsub, lookup + 4)):
                        sub = lookup + u16(gsub, lookup + 6 + 2 * n)
                        sub_kind = kind
                        if kind == 7:
                            sub_kind = u16(gsub, sub + 2)
                            sub += u32(gsub, sub + 4)
                        if sub_kind != 1:
                            raise SystemExit(f"the Russian locl has a lookup of type {sub_kind}, not a single substitution")
                        mapping.update(single_substitution(gsub, sub))
            return mapping
    raise SystemExit("no Russian language system in Cyrillic")


def read_cmap(cmap: bytes) -> tuple:
    """The character map's Unicode subtables (each format 4) and what they map: (records, {code: glyph})."""
    records = [(u16(cmap, 4 + 8 * i), u16(cmap, 6 + 8 * i), u32(cmap, 8 + 8 * i)) for i in range(u16(cmap, 2))]
    if {(p, e) for p, e, _ in records} != {(0, 3), (3, 1)} or any(u16(cmap, o) != 4 for _, _, o in records):
        raise SystemExit(f"unexpected character map subtables: {[(p, e, u16(cmap, o)) for p, e, o in records]}")
    sub = records[0][2]
    segments = u16(cmap, sub + 6) // 2
    ends, starts, deltas, offsets = sub + 14, sub + 16 + 2 * segments, sub + 16 + 4 * segments, sub + 16 + 6 * segments
    chars = {}
    for i in range(segments):
        start, end, delta, offset = u16(cmap, starts + 2 * i), u16(cmap, ends + 2 * i), u16(cmap, deltas + 2 * i), u16(cmap, offsets + 2 * i)
        for c in range(start, end + 1):
            if c == 0xFFFF:
                continue
            if offset == 0:
                glyph = (c + delta) & 0xFFFF
            else:
                glyph = u16(cmap, offsets + 2 * i + offset + 2 * (c - start))
                glyph = (glyph + delta) & 0xFFFF if glyph else 0
            if glyph:
                chars[c] = glyph
    return records, chars


def format4(chars: dict) -> bytes:
    """A format 4 subtable mapping `chars`: a segment for each run of characters whose glyphs
    follow one another, and the closing 0xFFFF."""
    segments = []
    for c in sorted(chars):
        if segments and c == segments[-1][1] + 1 and chars[c] - c == segments[-1][2]:
            segments[-1][1] = c
        else:
            segments.append([c, c, chars[c] - c])
    segments.append([0xFFFF, 0xFFFF, 1])
    count = len(segments)
    power = 1
    while power * 2 <= count:
        power *= 2
    body = struct.pack(f">{count}H", *[end for _, end, _ in segments]) + b"\0\0"
    body += struct.pack(f">{count}H", *[start for start, _, _ in segments])
    body += struct.pack(f">{count}H", *[delta & 0xFFFF for _, _, delta in segments])
    body += struct.pack(f">{count}H", *([0] * count))
    length = 14 + len(body)
    if length > 0xFFFF:
        raise SystemExit("the character map is too large for format 4")
    return struct.pack(">7H", 4, length, 0, count * 2, power * 2, power.bit_length() - 1, count * 2 - power * 2) + body


def standard_cyrillic(font: Font) -> str:
    """Maps the characters the Russian `locl` changes to its forms; returns them."""
    forms = russian_forms(font.tables[b"GSUB"])
    records, chars = read_cmap(font.tables[b"cmap"])
    changed = {c: forms[g] for c, g in chars.items() if g in forms}
    chars.update(changed)
    subtable = format4(chars)
    cmap = struct.pack(">HH", 0, len(records))
    for platform, encoding, _ in sorted(records):
        cmap += struct.pack(">HHI", platform, encoding, 4 + 8 * len(records))
    font.tables[b"cmap"] = cmap + subtable
    return "".join(chr(c) for c in sorted(changed))



def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    source, out = Path(sys.argv[1]), Path(sys.argv[2])
    for name, bundled in FONTS.items():
        font = Font((source / name).read_bytes())
        changed = standard_cyrillic(font)
        data = font.write()
        (out / bundled).write_bytes(data)
        print(f"{bundled}: {len(changed)} characters in their Russian forms ({changed}), {len(data):,} bytes")


if __name__ == "__main__":
    main()
