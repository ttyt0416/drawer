"""Wrap catalogued developer-owned UI literals; never rewrite arbitrary/user strings."""
import argparse
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
LITERAL = r'"(?:[^"\\]|\\.)*"'
PATTERNS = [
    re.compile(r'\bui\.(?:button|label|small|heading|menu_button|collapsing)\(\s*(?P<text>'+LITERAL+r')'),
    re.compile(r'\b(?:egui::)?(?:Button|RichText)::new\(\s*(?P<text>'+LITERAL+r')'),
    re.compile(r'\b(?:egui::)?ComboBox::from_label\(\s*(?P<text>'+LITERAL+r')'),
    re.compile(r'\bui\.(?:checkbox|selectable_label|colored_label)\(\s*[^,;"{}]+,\s*(?P<text>'+LITERAL+r')'),
    re.compile(r'\bui\.(?:radio_value|selectable_value)\(\s*[^,;"{}]+,\s*[^,;"{}]+,\s*(?P<text>'+LITERAL+r')'),
    re.compile(r'\bcrate::help::button\(\s*ui,\s*(?P<text>'+LITERAL+r')'),
    re.compile(r'\bcrate::help::(?:checkbox|selectable)\(\s*ui,\s*[^,;"{}]+,\s*(?P<text>'+LITERAL+r')'),
    re.compile(r'\.(?:on_hover_text|hint_text)\(\s*(?P<text>'+LITERAL+r')'),
]
WINDOW = re.compile(r'\b(?:egui::)?Window::new\(\s*(?P<text>'+LITERAL+r')\s*\)')


def catalog():
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError('Duplicate translation key: '+key)
            result[key] = value
        return result
    words = json.loads((ROOT/'locales/ko.json').read_text(encoding='utf-8'), object_pairs_hook=unique)
    if not all(isinstance(a,str) and isinstance(b,str) and a.strip() and b.strip() for a,b in words.items()):
        raise ValueError('Empty/invalid translation entry')
    return words


def replacements(source, words):
    edits = {}
    for pattern in PATTERNS:
        for match in pattern.finditer(source):
            literal = match['text']
            try:
                key = json.loads(literal)
            except ValueError:
                continue
            if key in words:
                edits[match.span('text')] = 'crate::i18n::text('+literal+')'
    for match in WINDOW.finditer(source):
        literal = match['text']
        if json.loads(literal) in words:
            edits[match.span()] = 'egui::Window::new(crate::i18n::text('+literal+')).id(egui::Id::new('+literal+'))'
    return edits


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    words = catalog()
    count = 0
    changed = 0
    for path in sorted((ROOT/'crates/drawer-app/src').glob('*.rs')):
        if path.name=='i18n.rs':
            continue
        source = path.read_text(encoding='utf-8')
        edits = replacements(source,words)
        if edits:
            changed += 1
            count += len(edits)
            if args.apply:
                for (start,end),replacement in sorted(edits.items(),reverse=True):
                    source = source[:start]+replacement+source[end:]
                path.write_text(source,encoding='utf-8',newline='\n')
    if count and not args.apply:
        raise SystemExit(f'{count} catalogued UI literals still lack localized rendering in {changed} files')
    print(f'{len(words)} Korean entries; '+(f'wrapped {count} literals in {changed} files' if args.apply else 'catalogued literal coverage passed; other text falls back to English'))


if __name__=='__main__':
    main()
