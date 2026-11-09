"""Writes scratch/i18n/chunks/topup.toml: the template's strings that the languages lack (those
`ta-tools i18n lang` added, empty, since they were translated), in the template's order under
their source files.

    python tools/i18n/topup.py [code ...]      (default: every language in lang/)
"""
import glob
import os
import sys
import tomllib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from split import ROOT, groups, quote  # noqa: E402

if __name__ == '__main__':
    codes = sys.argv[1:] or sorted(os.path.splitext(os.path.basename(p))[0] for p in glob.glob(os.path.join(ROOT, 'lang', '*.toml')) if not p.endswith('template.toml'))
    lacking = set()
    for code in codes:
        strings = tomllib.loads(open(os.path.join(ROOT, 'lang', code + '.toml'), encoding='utf-8').read()).get('strings', {})
        lacking.update(k for k, v in strings.items() if not str(v).strip())
    template = open(os.path.join(ROOT, 'lang', 'template.toml'), encoding='utf-8').read()
    lines = []
    count = 0
    for source, keys in groups(template):
        new = [k for k in keys if k in lacking]
        if new:
            lines += ['', '# ' + source] + [quote(k) + ' = ""' for k in new]
            count += len(new)
    out_dir = os.path.join(ROOT, 'scratch', 'i18n', 'chunks')
    os.makedirs(out_dir, exist_ok=True)
    content = '\n'.join(lines).strip('\n') + '\n'
    tomllib.loads(content)
    open(os.path.join(out_dir, 'topup.toml'), 'w', encoding='utf-8', newline='\n').write(content)
    print(count, 'strings to translate in scratch/i18n/chunks/topup.toml')
