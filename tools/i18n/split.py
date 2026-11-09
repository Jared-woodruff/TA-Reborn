"""Splits lang/template.toml into chunks of about N strings for translating:
scratch/i18n/chunks/chunk_NN.toml, each string on one line (a TOML basic string, newlines
as \\n), under `# <source file>` comments that say where the text appears.

    python tools/i18n/split.py [N]
"""
import os
import sys
import tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def quote(s):
    out = []
    for ch in s:
        if ch == '\\':
            out.append('\\\\')
        elif ch == '"':
            out.append('\\"')
        elif ch == '\n':
            out.append('\\n')
        elif ch == '\t':
            out.append('\\t')
        elif ord(ch) < 0x20:
            out.append('\\u%04x' % ord(ch))
        else:
            out.append(ch)
    return '"' + ''.join(out) + '"'


def groups(template):
    """(source file, [keys]) in the template's order."""
    body = template.split('[strings]', 1)[1]
    parts = ('\n' + body).split('\n# ')
    out = []
    for part in parts[1:]:
        header, _, rest = part.partition('\n')
        keys = list(tomllib.loads(rest).keys())
        if keys:
            out.append((header.strip(), keys))
    return out


if __name__ == '__main__':
    N = int(sys.argv[1]) if len(sys.argv) > 1 else 140
    template = open(os.path.join(ROOT, 'lang', 'template.toml'), encoding='utf-8').read()
    all_keys = list(tomllib.loads(template)['strings'].keys())
    out_dir = os.path.join(ROOT, 'scratch', 'i18n', 'chunks')
    os.makedirs(out_dir, exist_ok=True)
    for f in os.listdir(out_dir):
        os.remove(os.path.join(out_dir, f))
    chunks = []
    lines = []
    count = 0
    for source, keys in groups(template):
        for i, k in enumerate(keys):
            if count >= N:
                chunks.append(lines)
                lines, count = [], 0
            if i == 0 or not lines:
                lines.append('')
                lines.append('# ' + source)
            lines.append(quote(k) + ' = ""')
            count += 1
    if lines:
        chunks.append(lines)
    total = 0
    for i, c in enumerate(chunks, 1):
        content = '\n'.join(c).strip('\n') + '\n'
        open(os.path.join(out_dir, 'chunk_%02d.toml' % i), 'w', encoding='utf-8', newline='\n').write(content)
        total += len(tomllib.loads(content))
    assert total == len(all_keys), (total, len(all_keys))
    print(len(chunks), 'chunks,', total, 'strings, in', out_dir)
