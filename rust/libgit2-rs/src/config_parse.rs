//! Port of `src/libgit2/config_parse.c`: the parser of git's configuration
//! file format, shared by the file backend (reading and rewriting files)
//! and the memory backend.
//!
//! Only the config modules use it (no C that stays calls `git_config_parse`
//! or reads `git_config_parser`), so it is a Rust API alone: a
//! [`ConfigParser`] over a `&[u8]`, and a [`ParseHandler`] whose methods
//! are C's four callbacks. The control flow, the error messages and the
//! line numbers are the C's; in particular the C works on NUL-terminated
//! copies of each line (`git__strndup`), so a line is read only up to its
//! first NUL byte, as here.

use crate::git_assert;
use crate::parse::GitParseCtx;
use crate::util::ctype_compat::{isalnum, isspace, tolower};
use crate::util::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use crate::util::str::{Bom, detect_bom};

/// `git_config_escapes`: the characters that follow a backslash.
pub const ESCAPES: &[u8; 5] = b"ntb\"\\";
/// `git_config_escaped`: what each of [`ESCAPES`] stands for.
pub const ESCAPED: &[u8; 5] = b"\n\t\x08\"\\";

/// `git_config_parser`: the path named in error messages (C prints
/// `(null)` for none) and the parse context.
#[derive(Debug)]
pub struct ConfigParser<'a> {
    /// `path`.
    pub path: Option<&'a [u8]>,
    /// `ctx`.
    pub ctx: GitParseCtx<'a>,
}

/// The callbacks of `git_config_parse`. Each defaults to doing nothing,
/// which is what a NULL callback does in C. An error stops the parse and
/// is its result.
pub trait ParseHandler {
    /// `on_section`: a section header was parsed into `section` (the
    /// section lower-cased, then `.` and the subsection as written);
    /// `line` is the text of the header (from its first byte, or from where
    /// the previous construct on the same line ended).
    fn on_section(&mut self, section: &[u8], line: &[u8]) -> Result<()> {
        let _ = (section, line);
        Ok(())
    }

    /// `on_variable`: a variable `name` (as written) with `value` (NULL for
    /// a bare name), in `section` (none before the first header); `line` is
    /// its whole text, continuation lines included.
    fn on_variable(
        &mut self,
        section: Option<&[u8]>,
        name: &[u8],
        value: Option<&[u8]>,
        line: &[u8],
    ) -> Result<()> {
        let _ = (section, name, value, line);
        Ok(())
    }

    /// `on_comment`: a comment or blank line.
    fn on_comment(&mut self, line: &[u8]) -> Result<()> {
        let _ = line;
        Ok(())
    }

    /// `on_eof`: the end of the content, in `section`.
    fn on_eof(&mut self, section: Option<&[u8]>) -> Result<()> {
        let _ = section;
        Ok(())
    }
}

/// The handler that ignores everything (a parse for its errors only).
impl ParseHandler for () {}

/// Byte `i` of a NUL-terminated copy of `s`: 0 past its end.
#[inline(always)]
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// What `git__strndup` copies of `s`: up to its first NUL.
#[inline]
fn until_nul(s: &[u8]) -> &[u8] {
    match memchr::memchr(0, s) {
        Some(n) => &s[..n],
        None => s,
    }
}

/// `config_keychar` / `is_namechar`.
#[inline(always)]
fn is_namechar(c: u8) -> bool {
    isalnum(c) || c == b'-'
}

/// `strip_comments`: the length of `line` without its comment and the
/// whitespace before it, and the number of quotes seen (from `in_quotes`).
fn strip_comments(line: &[u8], in_quotes: i32) -> (usize, i32) {
    let mut quote_count = in_quotes;
    let mut backslash_count = 0u32;
    let mut i = 0;
    while i < line.len() {
        let c = line[i];
        if c == b'"' && (i == 0 || line[i - 1] != b'\\') {
            quote_count = quote_count.wrapping_add(1);
        }
        if (c == b';' || c == b'#') && quote_count % 2 == 0 && backslash_count % 2 == 0 {
            break;
        }
        if c == b'\\' {
            backslash_count += 1;
        } else {
            backslash_count = 0;
        }
        i += 1;
    }
    while i > 0 && isspace(line[i - 1]) {
        i -= 1;
    }
    (i, quote_count)
}

/// `unescape_line`: `ptr` without its quotes and with its escapes
/// replaced, and whether it ends in a backslash (a continued value).
/// Quotes are counted into `quote_count` when given.
fn unescape_line(ptr: &[u8], mut quote_count: Option<&mut i32>) -> Result<(Vec<u8>, bool)> {
    let mut out = Vec::new();
    out.try_reserve_exact(ptr.len() + 1)
        .map_err(|_| errors::oom())?;
    let mut i = 0;
    while i < ptr.len() {
        let c = ptr[i];
        if c == b'"' {
            if let Some(q) = quote_count.as_deref_mut() {
                *q = q.wrapping_add(1);
            }
        } else if c != b'\\' {
            out.push(c);
        } else {
            i += 1;
            if i == ptr.len() {
                return Ok((out, true));
            }
            match ESCAPES.iter().position(|&e| e == ptr[i]) {
                Some(k) => out.push(ESCAPED[k]),
                None => {
                    return Err(crate::git_error_set!(
                        ErrorClass::CONFIG,
                        "invalid escape at {}",
                        Bytes(&ptr[i..])
                    ));
                }
            }
        }
        i += 1;
    }
    Ok((out, false))
}

/// `parse_name`: the name at the start of `line` and the value after its
/// `=`, if any.
fn parse_name<'l>(
    parser: &ConfigParser<'_>,
    line: &'l [u8],
) -> Result<(&'l [u8], Option<&'l [u8]>)> {
    let name_end = line
        .iter()
        .position(|&c| !is_namechar(c))
        .unwrap_or(line.len());
    if name_end == 0 {
        return Err(parser.error(0, "invalid configuration key"));
    }
    let mut vs = name_end;
    while vs < line.len() && isspace(line[vs]) {
        vs += 1;
    }
    let value = if vs < line.len() {
        if line[vs] != b'=' {
            return Err(parser.error(0, "invalid configuration key"));
        }
        Some(&line[vs + 1..])
    } else {
        None
    };
    Ok((&line[..name_end], value))
}

/// A parsed variable: its name (as written) and value.
struct Variable<'l> {
    name: &'l [u8],
    value: Option<Vec<u8>>,
}

impl<'a> ConfigParser<'a> {
    /// `git_config_parser_init`: a parser over `data`, naming `path` in its
    /// errors.
    pub fn new(path: Option<&'a [u8]>, data: &'a [u8]) -> ConfigParser<'a> {
        ConfigParser {
            path,
            ctx: GitParseCtx::new(data),
        }
    }

    /// `set_parse_error`: the error, with the line number and, if `col` is
    /// not 0, the column.
    #[cold]
    fn error(&self, col: i32, what: &str) -> ErrorCode {
        let path = Bytes(self.path.unwrap_or(b"(null)"));
        let line = self.ctx.line_num();
        if col != 0 {
            crate::git_error_set!(
                ErrorClass::CONFIG,
                "failed to parse config file: {what} (in {path}:{line}, column {col})"
            )
        } else {
            crate::git_error_set!(
                ErrorClass::CONFIG,
                "failed to parse config file: {what} (in {path}:{line})"
            )
        }
    }

    /// `parse_subsection_header`: the section name `base` + `.` + the
    /// quoted subsection of `line` (from `pos`), and the offset in `line`
    /// past its `"]`.
    fn parse_subsection_header(
        &self,
        line: &[u8],
        mut pos: usize,
        base: &[u8],
    ) -> Result<(Vec<u8>, usize)> {
        while isspace(at(line, pos)) {
            pos += 1;
        }
        if at(line, pos) != b'"' {
            return Err(self.error(0, "missing quotation marks in section header"));
        }
        let first = pos;
        // There is one: `line[first]`.
        let last = memchr::memrchr(b'"', line).unwrap_or(first);
        let quoted_len = last - first;
        if last > i32::MAX as usize {
            return Err(self.error(0, "invalid section header, line too long"));
        }
        if quoted_len == 0 {
            return Err(self.error(0, "missing closing quotation mark in section header"));
        }
        let alloc_len = crate::util::integer::alloc_add(base.len(), quoted_len)?;
        let alloc_len = crate::util::integer::alloc_add(alloc_len, 2)?;
        let mut buf = Vec::new();
        buf.try_reserve_exact(alloc_len)
            .map_err(|_| errors::oom())?;
        buf.extend_from_slice(base);
        buf.push(b'.');

        let l = &line[first..];
        let lastrel = last - first;
        let mut rpos = 1usize;
        let mut c = at(l, rpos);
        loop {
            match c {
                0 => return Err(self.error(0, "unexpected end-of-line in section header")),
                b'"' => break,
                b'\\' => {
                    rpos += 1;
                    c = at(l, rpos);
                    if c == 0 {
                        return Err(
                            self.error(rpos as i32, "unexpected end-of-line in section header")
                        );
                    }
                }
                _ => {}
            }
            buf.push(c);
            rpos += 1;
            c = at(l, rpos);
            if rpos >= lastrel {
                break;
            }
        }
        if at(l, rpos) != b'"' || at(l, rpos + 1) != b']' {
            return Err(self.error(rpos as i32, "unexpected text after closing quotes"));
        }
        Ok((buf, first + rpos + 2))
    }

    /// `parse_section_header`: the section's name and the length of its
    /// header in the current line (after its leading whitespace).
    fn parse_section_header(&mut self) -> Result<(Vec<u8>, usize)> {
        let _ = self.ctx.advance_ws();
        let line = until_nul(self.ctx.line());
        let Some(name_end) = memchr::memrchr(b']', line) else {
            return Err(self.error(0, "missing ']' in section header"));
        };
        let mut name = Vec::new();
        name.try_reserve_exact(name_end + 1)
            .map_err(|_| errors::oom())?;
        let mut pos = 0usize;
        let c = at(line, pos);
        pos += 1;
        git_assert!(c == b'[', "c == '['");
        let mut c = at(line, pos);
        pos += 1;
        loop {
            if isspace(c) {
                return self.parse_subsection_header(line, pos, &name);
            }
            if !is_namechar(c) && c != b'.' {
                return Err(self.error(pos as i32, "unexpected character in header"));
            }
            name.push(tolower(c));
            c = at(line, pos);
            pos += 1;
            if c == b']' {
                break;
            }
        }
        Ok((name, pos))
    }

    /// `skip_bom`.
    fn skip_bom(&mut self) {
        let (bom, len) = detect_bom(self.ctx.content());
        if bom == Bom::Utf8 {
            self.ctx.advance_chars(len);
        }
    }

    /// `parse_multiline_variable`: append the continuation lines of a value
    /// to `value`, adding their lengths to `line_len`.
    fn parse_multiline_variable(
        &mut self,
        value: &mut Vec<u8>,
        mut in_quotes: i32,
        line_len: &mut usize,
    ) -> Result<()> {
        let mut multiline = true;
        while multiline {
            self.ctx.advance_line();
            let line = until_nul(self.ctx.line());
            *line_len = line_len
                .checked_add(self.ctx.line_len())
                .ok_or(ErrorCode::GENERIC)?;
            // The end of the content: no continuation (not an error).
            if line.is_empty() {
                return Ok(());
            }
            let (len, quote_count) = strip_comments(line, in_quotes);
            // A comment alone: as if the line were not there.
            if len == 0 {
                in_quotes = quote_count;
                continue;
            }
            let (proc_line, more) = unescape_line(&line[..len], Some(&mut in_quotes))?;
            multiline = more;
            value
                .try_reserve(proc_line.len())
                .map_err(|_| errors::oom())?;
            value.extend_from_slice(&proc_line);
            in_quotes = quote_count;
        }
        Ok(())
    }

    /// `parse_variable`: the variable of the current line (and of its
    /// continuation lines, whose lengths are added to `line_len`).
    fn parse_variable(&mut self, line_len: &mut usize) -> Result<Variable<'a>> {
        let _ = self.ctx.advance_ws();
        let line: &'a [u8] = until_nul(self.ctx.line());
        let (len, quote_count) = strip_comments(line, 0);
        let (name, value_start) = parse_name(self, &line[..len])?;
        let mut value = None;
        if let Some(mut vs) = value_start {
            while !vs.is_empty() && isspace(vs[0]) {
                vs = &vs[1..];
            }
            let (mut v, multi) = unescape_line(vs, None)?;
            if multi
                && self
                    .parse_multiline_variable(&mut v, quote_count % 2, line_len)
                    .is_err()
            {
                return Err(ErrorCode::GENERIC);
            }
            value = Some(v);
        }
        Ok(Variable { name, value })
    }

    /// `git_config_parse`: parse the content, calling `handler`'s methods;
    /// the first error (from the parser or a callback) stops it.
    pub fn parse<H: ParseHandler + ?Sized>(&mut self, handler: &mut H) -> Result<()> {
        let mut current_section: Option<Vec<u8>> = None;
        let mut result: Result<()> = Ok(());

        self.skip_bom();

        while self.ctx.remain_len() > 0 {
            let mut restart = true;
            while restart {
                restart = false;
                let line_start = self.ctx.offset();
                let mut line_len = self.ctx.line_len();

                // The first non-whitespace character, or, if there is none,
                // the first whitespace one: whitespace is kept when the file
                // is written back.
                let Some(c) = self.ctx.peek(true).or_else(|| self.ctx.peek(false)) else {
                    break;
                };

                match c {
                    b'[' => {
                        current_section = None;
                        match self.parse_section_header() {
                            Err(e) => result = Err(e),
                            Ok((section, consumed)) => {
                                self.ctx.advance_chars(consumed);
                                let line = &self.ctx.content()[line_start..][..line_len];
                                result = handler.on_section(&section, line);
                                current_section = Some(section);
                                // The rest of the line, if any, is parsed
                                // as a line of its own (the result above is
                                // then lost, as in C).
                                if self.ctx.peek(true).is_some() {
                                    restart = true;
                                    continue;
                                }
                            }
                        }
                    }
                    b'\n' | b'\r' | b' ' | b'\t' | b';' | b'#' => {
                        let line = &self.ctx.content()[line_start..][..line_len];
                        result = handler.on_comment(line);
                    }
                    _ => {
                        result = match self.parse_variable(&mut line_len) {
                            Err(e) => Err(e),
                            Ok(var) => {
                                let content = self.ctx.content();
                                let end = (line_start + line_len).min(content.len());
                                handler.on_variable(
                                    current_section.as_deref(),
                                    var.name,
                                    var.value.as_deref(),
                                    &content[line_start..end],
                                )
                            }
                        }
                    }
                }
                result?;
            }
            self.ctx.advance_line();
        }

        let _ = result;
        handler.on_eof(current_section.as_deref())
    }
}

/// `git_config_parse` of `data` with `handler`, as C's
/// `git_config_parser_init` + `git_config_parse` + `git_config_parser_dispose`.
pub fn parse<H: ParseHandler + ?Sized>(
    path: Option<&[u8]>,
    data: &[u8],
    handler: &mut H,
) -> Result<()> {
    ConfigParser::new(path, data).parse(handler)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Collect {
        events: Vec<String>,
    }

    fn s(b: &[u8]) -> String {
        String::from_utf8_lossy(b).into_owned()
    }

    impl ParseHandler for Collect {
        fn on_section(&mut self, section: &[u8], line: &[u8]) -> Result<()> {
            self.events.push(format!("S {} |{}|", s(section), s(line)));
            Ok(())
        }
        fn on_variable(
            &mut self,
            section: Option<&[u8]>,
            name: &[u8],
            value: Option<&[u8]>,
            line: &[u8],
        ) -> Result<()> {
            self.events.push(format!(
                "V {:?} {} {:?} |{}|",
                section.map(s),
                s(name),
                value.map(s),
                s(line)
            ));
            Ok(())
        }
        fn on_comment(&mut self, line: &[u8]) -> Result<()> {
            self.events.push(format!("C |{}|", s(line)));
            Ok(())
        }
        fn on_eof(&mut self, section: Option<&[u8]>) -> Result<()> {
            self.events.push(format!("E {:?}", section.map(s)));
            Ok(())
        }
    }

    fn run(input: &[u8]) -> (Vec<String>, Option<String>) {
        let _g = crate::util::test_support::init();
        let mut c = Collect::default();
        let r = parse(Some(b"f"), input, &mut c);
        let err = r.err().map(|_| s(&errors::last().1));
        (c.events, err)
    }

    #[test]
    fn sections_and_variables() {
        let (ev, err) = run(b"[Core]\n\tBare = false ; c\n[remote \"Origin\"] url = x\n# c\nk\n");
        assert!(err.is_none());
        assert_eq!(
            ev,
            [
                "S core |[Core]\n|",
                "V Some(\"core\") Bare Some(\"false\") |\tBare = false ; c\n|",
                "S remote.Origin |[remote \"Origin\"] url = x\n|",
                "V Some(\"remote.Origin\") url Some(\"x\") | url = x\n|",
                "C |# c\n|",
                "V Some(\"remote.Origin\") k None |k\n|",
                "E Some(\"remote.Origin\")",
            ]
        );
    }

    #[test]
    fn multiline_and_escapes() {
        // A blank or comment line does not end a continued value.
        let (ev, err) = run(b"[a]\nb = one \\\n two \"q;\" \\t\\\n# c\nend\nc=\"x\\\"y\"\n");
        assert!(err.is_none(), "{err:?}");
        assert_eq!(
            ev[1],
            "V Some(\"a\") b Some(\"one  two q; \\tend\") |b = one \\\n two \"q;\" \\t\\\n# c\nend\n|"
        );
        assert_eq!(ev[2], "V Some(\"a\") c Some(\"x\\\"y\") |c=\"x\\\"y\"\n|");
    }

    #[test]
    fn errors_match_the_c() {
        let cases: &[(&[u8], &str)] = &[
            (
                b"[a\n",
                "failed to parse config file: missing ']' in section header (in f:1)",
            ),
            (
                b"[a!]\n",
                "failed to parse config file: unexpected character in header (in f:1, column 3)",
            ),
            (
                b"[a b]\n",
                "failed to parse config file: missing quotation marks in section header (in f:1)",
            ),
            (
                b"[a \"b]\n",
                "failed to parse config file: missing closing quotation mark in section header (in f:1)",
            ),
            (
                b"[a \"b\" c]\n",
                "failed to parse config file: unexpected text after closing quotes (in f:1, column 2)",
            ),
            (
                b"[a]\n\n=x\n",
                "failed to parse config file: invalid configuration key (in f:3)",
            ),
            (
                b"[a]\nb c\n",
                "failed to parse config file: invalid configuration key (in f:2)",
            ),
            (b"[a]\nb=\\q\n", "invalid escape at q"),
        ];
        for (input, msg) in cases {
            let (_, err) = run(input);
            assert_eq!(err.as_deref(), Some(*msg), "{}", s(input));
        }
    }

    #[test]
    fn bom_and_nul() {
        let (ev, err) = run(b"\xef\xbb\xbf[a]\nb=c\0d\n");
        assert!(err.is_none());
        assert_eq!(ev[1], "V Some(\"a\") b Some(\"c\") |b=c\0d\n|");
    }

    #[test]
    fn strip() {
        assert_eq!(strip_comments(b"a = b ; c", 0), (5, 0));
        assert_eq!(strip_comments(b"a = \"b ; c\"", 0), (11, 2));
        assert_eq!(strip_comments(b"a = b \\; c", 0), (10, 0));
        assert_eq!(strip_comments(b"x  ", 1), (1, 1));
    }
}
