"""Builds lang/<code>.toml from its translations so far and translated chunks, and checks it.

    python tools/i18n/merge.py <code> "<language name>"

Starts from lang/<code>.toml (when there is one), then takes every scratch/i18n/<code>/*.toml
(chunks: keys copied from scratch/i18n/chunks/, values translated) over it, writes
lang/<code>.toml with every template string in the template's order, and reports: strings
missing or left empty, keys that are not in the template, and translations whose
{placeholders} differ from the English. Exit code 0 when clean. Afterwards
`ta-tools i18n lang` puts the template's source-file comments back in.
"""
import os
import re
import sys
import tomllib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from split import ROOT, quote  # noqa: E402

code, name = sys.argv[1], sys.argv[2]
template = open(os.path.join(ROOT, 'lang', 'template.toml'), encoding='utf-8').read()
keys = list(tomllib.loads(template)['strings'].keys())
key_set = set(keys)

translated = {}
existing = os.path.join(ROOT, 'lang', code + '.toml')
if os.path.isfile(existing):
    for k, v in tomllib.loads(open(existing, encoding='utf-8').read()).get('strings', {}).items():
        if isinstance(v, str) and v.strip() and k in key_set:
            translated[k] = v

src_dir = os.path.join(ROOT, 'scratch', 'i18n', code)
problems = []
for f in sorted(os.listdir(src_dir)) if os.path.isdir(src_dir) else []:
    if not f.endswith('.toml'):
        continue
    try:
        part = tomllib.loads(open(os.path.join(src_dir, f), encoding='utf-8').read())
    except tomllib.TOMLDecodeError as e:
        problems.append('%s: not valid TOML: %s' % (f, e))
        continue
    if isinstance(part.get('strings'), dict):
        part = part['strings']
    for k, v in part.items():
        if not isinstance(v, str):
            problems.append('%s: the value of %r is not a string' % (f, k))
        elif k not in key_set:
            problems.append('%s: not in the template (copy the key exactly): %r' % (f, k))
        elif v.strip():
            translated[k] = v

placeholder = re.compile(r'\{[A-Za-z_][A-Za-z0-9_]*\}')
missing = [k for k in keys if not translated.get(k, '').strip()]
for k in keys:
    v = translated.get(k)
    if v and sorted(placeholder.findall(k)) != sorted(placeholder.findall(v)):
        problems.append('placeholders differ: %r -> %r' % (k, v))

lines = ['[language]', 'name = ' + quote(name), '', '[strings]']
lines += [quote(k) + ' = ' + quote(translated.get(k, '')) for k in keys]
out = '\n'.join(lines) + '\n'
tomllib.loads(out)
open(existing, 'w', encoding='utf-8', newline='\n').write(out)

print('%s (%s): %d of %d strings translated' % (code, name, len(keys) - len(missing), len(keys)))
if missing:
    print('MISSING %d, e.g.:' % len(missing))
    for k in missing[:20]:
        print('   ', quote(k))
for p in problems[:50]:
    print('PROBLEM', p)
sys.exit(1 if missing or problems else 0)
