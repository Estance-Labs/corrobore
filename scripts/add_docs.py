# Copyright (c) 2026 AreDee-Bangs
# SPDX-License-Identifier: MIT
#
# Permission is hereby granted, free of charge, to any person obtaining a copy
# of this software and associated documentation files (the "Software"), to deal
# in the Software without restriction, including without limitation the rights
# to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
# copies of the Software, and to permit persons to whom the Software is
# furnished to do so, subject to the following conditions:
#
# The above copyright notice and this permission notice shall be included in
# all copies or substantial portions of the Software.
#
# THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
# IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
# FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
# AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
# LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
# OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
# THE SOFTWARE.
#!/usr/bin/env python3
"""Add missing doc comments to undocumented public Rust items.

Handles:
- Crate-level //! module docs
- pub enum variants
- pub struct fields
- pub struct/enum/trait definitions
- pub fn / pub fn methods
"""

import re
import sys
from pathlib import Path
from typing import Optional


def humanize_name(name: str) -> str:
    """Convert CamelCase or snake_case to a readable phrase."""
    # CamelCase → words
    words = re.sub(r'(?<=[a-z])(?=[A-Z])', ' ', name)
    words = re.sub(r'(?<=[A-Z])(?=[A-Z][a-z])', ' ', words)
    # snake_case → words
    words = words.replace('_', ' ')
    return words.lower()


def needs_doc(lines: list[str], i: int) -> bool:
    """Check if line i has no doc comment (/// or //!) above it."""
    if i == 0:
        return True
    j = i - 1
    while j >= 0 and lines[j].strip() == '':
        j -= 1
    if j < 0:
        return True
    stripped = lines[j].strip()
    # Check if preceded by doc comment, derive macro, or attribute
    if stripped.startswith('///'):
        return False
    if stripped.startswith('#[') or stripped.startswith('//'):
        # Look above the attribute/comment for doc comments
        k = j - 1
        while k >= 0:
            s = lines[k].strip()
            if s.startswith('///'):
                return False
            if s.startswith('#[') or s == '' or s.startswith('//'):
                k -= 1
                continue
            break
        return True
    return True


def get_indent(line: str) -> str:
    """Get leading whitespace."""
    return line[:len(line) - len(line.lstrip())]


def add_docs_to_file(filepath: str) -> bool:
    """Add missing doc comments to a single file. Returns True if modified."""
    with open(filepath, 'r') as f:
        lines = f.readlines()
    
    original_len = len(lines)
    result = []
    i = 0
    modified = False
    
    # Check for missing crate-level doc
    has_crate_doc = any(line.strip().startswith('//!') for line in lines[:20])
    
    while i < len(lines):
        line = lines[i]
        stripped = line.strip()
        indent = get_indent(line)
        
        # --- pub enum ---
        m = re.match(r'^(\s*)pub enum (\w+)', line)
        if m and needs_doc(result + lines[:0], len(result)):
            doc = f'{indent}/// {humanize_name(m.group(2)).capitalize().rstrip(".")}.\n'
            result.append(doc)
            modified = True
            result.append(line)
            i += 1
            continue
        
        # --- pub struct ---
        m = re.match(r'^(\s*)pub struct (\w+)', line)
        if m and needs_doc(result + lines[:0], len(result)):
            doc = f'{indent}/// {humanize_name(m.group(2)).capitalize().rstrip(".")}.\n'
            result.append(doc)
            modified = True
            result.append(line)
            i += 1
            continue
        
        # --- pub trait ---
        m = re.match(r'^(\s*)pub trait (\w+)', line)
        if m and needs_doc(result + lines[:0], len(result)):
            doc = f'{indent}/// {humanize_name(m.group(2)).capitalize().rstrip(".")}.\n'
            result.append(doc)
            modified = True
            result.append(line)
            i += 1
            continue
        
        # --- pub type ---
        m = re.match(r'^(\s*)pub type (\w+)', line)
        if m and needs_doc(result + lines[:0], len(result)):
            doc = f'{indent}/// Type alias for [`{m.group(2)}`].\n'
            result.append(doc)
            modified = True
            result.append(line)
            i += 1
            continue
        
        # --- enum variants (inside enum block) ---
        # Match both simple variants and struct-style variants
        m_variant = re.match(r'^(\s+)(\w+)([,({]|\s*\{)', line)
        if m_variant and not stripped.startswith('//') and not stripped.startswith('pub') and not stripped.startswith('fn') and not stripped.startswith('let') and not stripped.startswith('use') and not stripped.startswith('mod') and not stripped.startswith('impl') and not stripped.startswith('type') and not stripped.startswith('const') and not stripped.startswith('static') and not stripped.startswith('where') and not stripped.startswith('self') and not stripped.startswith('return') and not stripped.startswith('if') and not stripped.startswith('for') and not stripped.startswith('match') and not stripped.startswith('else') and not stripped.startswith('}'):
            variant_name = m_variant.group(2)
            # Check if this is actually an enum variant (look for enum context)
            is_variant = False
            brace_depth = 0
            for prev_line in reversed(result[-60:]):
                ps = prev_line.strip()
                # Count braces to track nesting depth
                brace_depth += ps.count('}') - ps.count('{')
                if (ps.startswith('pub enum ') or ps.startswith('enum ')) and brace_depth <= 0:
                    is_variant = True
                    break
                if (ps.startswith('pub struct ') or ps.startswith('struct ') or ps.startswith('impl ') or ps.startswith('fn ') or ps.startswith('pub fn ')) and brace_depth <= 0:
                    break
            
            if is_variant and needs_doc(result, len(result)):
                # Only add if the variant name looks like a proper variant (starts uppercase)
                if variant_name[0].isupper():
                    doc = f'{indent}/// {humanize_name(variant_name).capitalize().rstrip(".")}.\n'
                    result.append(doc)
                    modified = True
        
        # --- pub struct fields ---
        m = re.match(r'^(\s+)pub (\w+): (.+)', line)
        if m and not stripped.startswith('pub fn') and not stripped.startswith('pub enum') and not stripped.startswith('pub struct') and not stripped.startswith('pub trait') and not stripped.startswith('pub mod') and not stripped.startswith('pub use') and not stripped.startswith('pub type') and not stripped.startswith('pub static') and not stripped.startswith('pub const'):
            field_name = m.group(2)
            if needs_doc(result, len(result)):
                doc = f'{indent}/// {humanize_name(field_name).capitalize().rstrip(".")}.\n'
                result.append(doc)
                modified = True
        
        # --- non-pub fields inside enum variants (e.g. field_name: Type inside struct variant) ---
        m_field = re.match(r'^(\s+)(\w+): (.+)', line)
        if m_field and not stripped.startswith('pub') and not stripped.startswith('//') and not stripped.startswith('fn') and not stripped.startswith('let') and not stripped.startswith('use') and not stripped.startswith('mod') and not stripped.startswith('impl') and not stripped.startswith('self') and not stripped.startswith('return') and not stripped.startswith('if') and not stripped.startswith('for') and not stripped.startswith('match') and not stripped.startswith('else') and not stripped.startswith('}') and not stripped.startswith('type') and not stripped.startswith('where') and not stripped.startswith('#'):
            field_name = m_field.group(2)
            # Check if we're inside an enum variant (struct-style)
            is_in_variant = False
            brace_depth = 0
            for prev_line in reversed(result[-30:]):
                ps = prev_line.strip()
                brace_depth += ps.count('}') - ps.count('{')
                if brace_depth < 0:
                    # We're inside a { block - check what opened it
                    # Look for a variant-like pattern (starts with uppercase)
                    m_v = re.match(r'\w+\s*\{', ps) or re.match(r'///.*', ps)
                    for check_line in reversed(result[-60:]):
                        cs = check_line.strip()
                        if re.match(r'[A-Z]\w*\s*\{', cs):
                            is_in_variant = True
                            break
                        if cs.startswith('pub enum') or cs.startswith('enum'):
                            is_in_variant = True
                            break
                        if cs.startswith('pub struct') or cs.startswith('struct') or cs.startswith('impl') or cs.startswith('fn') or cs.startswith('pub fn'):
                            break
                    break
            
            if is_in_variant and needs_doc(result, len(result)):
                doc = f'{indent}/// {humanize_name(field_name).capitalize().rstrip(".")}.\n'
                result.append(doc)
                modified = True
        
        # --- pub fn (standalone) ---
        m = re.match(r'^(\s*)pub fn (\w+)', line)
        if m and needs_doc(result, len(result)):
            fn_name = m.group(2)
            human = humanize_name(fn_name)
            # Determine verb-first doc
            if fn_name.startswith('new'):
                doc = f'{indent}/// Creates a new instance.\n'
            elif fn_name.startswith('from_'):
                doc = f'{indent}/// Creates an instance from {human.replace("from ", "")}.\n'
            elif fn_name.startswith('as_'):
                doc = f'{indent}/// Returns the value as {human.replace("as ", "")}.\n'
            elif fn_name.startswith('is_'):
                doc = f'{indent}/// Returns `true` if {human.replace("is ", "")}.\n'
            elif fn_name.startswith('has_'):
                doc = f'{indent}/// Returns `true` if has {human.replace("has ", "")}.\n'
            elif fn_name.startswith('get_') or fn_name.startswith('read_'):
                doc = f'{indent}/// Returns the {human.replace("get ", "").replace("read ", "")}.\n'
            elif fn_name.startswith('set_') or fn_name.startswith('with_'):
                doc = f'{indent}/// Sets the {human.replace("set ", "").replace("with ", "")}.\n'
            elif fn_name.startswith('validate_') or fn_name.startswith('check_'):
                doc = f'{indent}/// Validates the {human.replace("validate ", "").replace("check ", "")}.\n'
            elif fn_name.startswith('create_') or fn_name.startswith('build_'):
                doc = f'{indent}/// Creates the {human.replace("create ", "").replace("build ", "")}.\n'
            else:
                doc = f'{indent}/// {human.capitalize().rstrip(".")}.\n'
            result.append(doc)
            modified = True
        
        result.append(line)
        i += 1
    
    if modified:
        with open(filepath, 'w') as f:
            f.writelines(result)
    
    return modified


def main():
    crates_dir = Path(__file__).resolve().parent.parent / "crates"
    
    modified = []
    unchanged = []
    
    for rs_file in sorted(crates_dir.rglob("src/**/*.rs")):
        filepath = str(rs_file)
        if add_docs_to_file(filepath):
            modified.append(filepath)
        else:
            unchanged.append(filepath)
    
    # Also process lib.rs files directly
    for rs_file in sorted(crates_dir.rglob("src/lib.rs")):
        filepath = str(rs_file)
        if filepath not in modified and filepath not in unchanged:
            if add_docs_to_file(filepath):
                modified.append(filepath)
    
    import os
    print(f"\nModified: {len(modified)} files")
    print(f"Unchanged: {len(unchanged)} files")
    
    for f in modified:
        rel = os.path.relpath(f, str(crates_dir.parent))
        print(f"  M {rel}")


if __name__ == "__main__":
    main()
