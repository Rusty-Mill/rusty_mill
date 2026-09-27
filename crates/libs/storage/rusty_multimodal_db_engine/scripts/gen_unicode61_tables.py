#!/usr/bin/env python3
"""Regenerate src/fulltext/unicode_tables.rs from the SQLite amalgamation
libsqlite3-sys bundles, so the engine's unicode61 tokenizer matches FTS5.

Usage: python3 scripts/gen_unicode61_tables.py src/fulltext/unicode_tables.rs
"""
import re, glob, sys
src = open(glob.glob('/root/.cargo/registry/src/*/libsqlite3-sys-0.37.0/sqlite3/sqlite3.c')[0]).read()
version = re.search(r'#define SQLITE_VERSION\s+"([^"]+)"', src).group(1)

def body(pattern):
    m = re.search(pattern + r'\s*=\s*\{(.*?)\};', src, re.S)
    assert m, pattern
    return m.group(1)

def nums(text):
    text = re.sub(r'/\*.*?\*/', '', text, flags=re.S)
    return [int(x) for x in re.findall(r'-?\d+', text)]

def chars(text):
    out = []
    for tok in re.findall(r"'(\\0|.)'(\|HIBIT)?", text):
        ch, hi = tok
        v = 0 if ch == '\\0' else ord(ch)
        if hi: v |= 0x80
        out.append(v)
    return out

dia = nums(body(r'unsigned short aDia\[\]'))
achar = chars(body(r'unsigned char aChar\[\]'))
entries = re.findall(r'\{(\d+), (\d+), (\d+)\}', body(r'\} aEntry\[\]'))
aioff = nums(body(r'static const unsigned short aiOff\[\]'))
block = nums(body(r'static u16 aFts5UnicodeBlock\[\]'))
umap = nums(body(r'static u16 aFts5UnicodeMap\[\]'))
udata = nums(body(r'static u16 aFts5UnicodeData\[\]'))
assert len(dia) == len(achar), (len(dia), len(achar))
assert len(umap) == len(udata)

def arr(name, ty, vals, per=12):
    lines = []
    for i in range(0, len(vals), per):
        lines.append('    ' + ', '.join(str(v) for v in vals[i:i+per]) + ',')
    return f'pub(super) const {name}: [{ty}; {len(vals)}] = [\n' + '\n'.join(lines) + '\n];\n'

out = f'''//! Unicode tables for the `unicode61` tokenizer, generated from SQLite
//! {version}'s amalgamation (`fts5_unicode2.c`, itself generated from the
//! Unicode Character Database). SQLite is in the public domain.
//!
//! Do not edit by hand: regenerate with the script described in
//! `super::unicode61`'s module docs, so the tokenizer keeps matching the
//! FTS5 build the node's differential tests run against.

#![allow(clippy::unreadable_literal)]

/// `SQLITE_VERSION` of the amalgamation these tables came from.
pub(super) const SQLITE_VERSION: &str = "{version}";

'''
out += arr('DIACRITIC_KEYS', 'u16', dia)
out += '\n' + arr('DIACRITIC_BASE', 'u8', achar)
out += '\n/// `(first code point, flags, range length)`.\n'
out += f'pub(super) const FOLD_RULES: [(u16, u8, u8); {len(entries)}] = [\n' + '\n'.join(
    '    ' + ' '.join(f'({a}, {b}, {c}),' for a, b, c in entries[i:i+4]) for i in range(0, len(entries), 4)) + '\n];\n'
out += '\n' + arr('FOLD_OFFSETS', 'u16', aioff)
out += '\n' + arr('CATEGORY_BLOCKS', 'u16', block)
out += '\n' + arr('CATEGORY_MAP', 'u16', umap)
out += '\n' + arr('CATEGORY_DATA', 'u16', udata)
open(sys.argv[1], 'w').write(out)
print(version, len(dia), len(entries), len(aioff), len(block), len(umap))
