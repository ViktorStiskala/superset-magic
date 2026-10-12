//! The `orca.yaml` text editor: make sure `scripts.setup` runs the
//! `magic.sh sync` line and set `worktree.sharedDirectories`, changing nothing
//! else in the file – or refuse.
//!
//! Orca reads `orca.yaml` from the main checkout. Two of its keys are ss-magic's
//! business, and everything else (`defaultTabs`, `environmentRecipes`,
//! `issueCommand`, comments, blank lines, the indentation style) belongs to
//! whoever wrote the file. No Rust YAML crate round-trips a document while
//! keeping its comments, so the file is never re-serialized. It is edited as
//! TEXT (KTD2): `saphyr` parses the current text and reports where each node
//! sits, new text is spliced in at those positions, and the result is parsed
//! again and compared with what was intended before it is accepted. That second
//! parse is the arbiter: a splice that lands wrong produces either invalid YAML
//! or a value other than the intended one, and both are a refusal rather than a
//! write.
//!
//! [`edit`] is pure – text in, text or a [`Refusal`] out, no I/O – so the whole
//! decision is unit-testable against a fixture table.
//!
//! ## What `saphyr`'s spans can and cannot be trusted for
//!
//! Positions are CHARACTER offsets into the parsed text, not bytes, so this
//! module works on a `Vec<char>`. Measured against saphyr 0.2.1:
//!
//! - A KEY's span and a PLAIN scalar's span are exact.
//! - A QUOTED scalar's span can run past the closing quote, over trailing spaces
//!   and a same-line comment. Its end is found by scanning for the closing
//!   quote instead ([`Source::scan_double_quoted`], [`Source::scan_single_quoted`]).
//! - A FLOW collection's span stops before its closing bracket. Its end is found
//!   by matching brackets ([`Source::scan_flow`]).
//! - A BLOCK scalar's span starts on its first content line, not at the `|`, and
//!   ends at the first less-indented text after it. A BLOCK sequence's span ends
//!   at the next token, past any comments in between. Both are walked back to
//!   the last line that actually belongs to the value.
//! - An alias is handed over already resolved to a copy of the anchored node; its
//!   own span covers the `*name` token.
//!
//! None of that has to be right for the result to be safe – the re-parse check
//! catches a wrong guess – but getting it right is what keeps a user's comments
//! where they were.

use saphyr::{LoadableYamlNode, MarkedYaml, Scalar, YamlData};

/// The line `scripts.setup` must contain so a new Orca worktree runs ss-magic's
/// forward sync, exactly as a Superset workspace does through `config.json`.
pub const SETUP_LINE: &str = "./.superset/magic.sh sync";

const SCRIPTS: &str = "scripts";
const SETUP: &str = "setup";
const WORKTREE: &str = "worktree";
const SHARED_DIRECTORIES: &str = "sharedDirectories";

/// Indentation used when the file gives nothing to copy it from.
const DEFAULT_INDENT_UNIT: usize = 2;

/// Why `orca.yaml` was left alone, plus what the person can paste in by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// One sentence fragment naming what about the file ss-magic cannot edit
    /// safely, worded to follow "orca.yaml was not changed: ".
    pub reason: String,
    /// The managed block exactly as ss-magic would have written it into an
    /// empty file: the setup line (when it was to be ensured) and the directory
    /// list.
    pub manual_block: String,
}

/// Produce the new text of `orca.yaml`.
///
/// `current` is the file's present text, or `None` when there is no file.
/// `shared_dirs` REPLACES `worktree.sharedDirectories` outright – ss-magic owns
/// that list (R21), so an entry someone added by hand does not survive.
/// `ensure_setup` decides whether `scripts.setup` is looked at all: when set,
/// the [`SETUP_LINE`] is added if no line of the script already equals it
/// (prepended, so secrets land before any `pnpm install` in the same script);
/// when clear, `scripts` is left exactly as it is, which is how a refresh avoids
/// re-adding a line someone removed on purpose (R25).
///
/// The returned text equals `current` byte for byte when nothing needed to
/// change, so a caller can compare the two to decide whether to write.
///
/// A [`Refusal`] means the file must be left untouched. Whole-document problems
/// always refuse, because Orca itself would ignore or misread such a file: text
/// that does not parse (duplicate keys included), more than one document, a
/// `%YAML` directive other than 1.2, or a top level that is not a block mapping.
/// A problem with one managed key refuses only when that key actually needs an
/// edit: a parent (`scripts`, `worktree`) that is a flow mapping, a null, an
/// alias or otherwise not a block mapping; an alias or merge key on the path;
/// or a `scripts.setup` that is not a string.
///
/// An ANCHOR or a TAG on a managed node is not a refusal on its own. It is left
/// in place and the edit goes ahead; whether that was safe is decided the same
/// way as everything else, by parsing the result. When some other key refers to
/// the anchor, that key's value changes with the edit, the check sees it, and
/// the file is refused then.
pub fn edit(
    current: Option<&str>,
    shared_dirs: &[String],
    ensure_setup: bool,
) -> Result<String, Refusal> {
    let refuse = |reason: String| Refusal {
        reason,
        manual_block: managed_block(shared_dirs, ensure_setup),
    };

    let original = current.unwrap_or("");
    // saphyr does not strip a byte-order mark; it would become part of the first
    // key. Parse without it and put it back on the way out.
    let (bom, body) = match original.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", original),
    };

    let docs = MarkedYaml::load_from_str(body)
        .map_err(|err| refuse(format!("it is not valid YAML ({err})")))?;
    if docs.len() > 1 {
        return Err(refuse(
            "it holds more than one YAML document".to_string(),
        ));
    }
    if let Some(version) = foreign_yaml_directive(body) {
        return Err(refuse(format!(
            "it declares `%YAML {version}`, and ss-magic only edits YAML 1.2"
        )));
    }

    let src = Source::new(body);
    let root = docs.first();
    let plan = Plan::build(&src, root, shared_dirs, ensure_setup).map_err(&refuse)?;
    if plan.is_noop() {
        return Ok(original.to_string());
    }

    // The setup value can be written more than one way. A block scalar reads
    // best but cannot carry every string; a double-quoted scalar can carry
    // anything. Each candidate is built and then judged by the re-parse check,
    // so the first one that provably yields the intended values wins.
    for style in [
        SetupStyle::Block,
        SetupStyle::BlockUnpadded,
        SetupStyle::Quoted,
    ] {
        let Some(edits) = plan.edits(&src, style) else {
            continue;
        };
        let candidate = src.apply(&edits);
        if verify(root, &candidate, plan.intended_setup.as_deref(), shared_dirs).is_ok() {
            return Ok(format!("{bom}{candidate}"));
        }
        if !plan.setup_style_matters() {
            break;
        }
    }
    Err(refuse(
        "its layout could not be edited safely (the edited text did not parse back to the \
         intended values)"
            .to_string(),
    ))
}

/// The managed block as ss-magic writes it into an empty file, with `\n` line
/// endings. Shown to the person when the file cannot be edited, and the text a
/// brand-new `orca.yaml` consists of.
pub fn managed_block(shared_dirs: &[String], ensure_setup: bool) -> String {
    let mut out = String::new();
    if ensure_setup {
        out.push_str(&scripts_block(0, DEFAULT_INDENT_UNIT, "\n"));
    }
    out.push_str(&worktree_block(shared_dirs, 0, DEFAULT_INDENT_UNIT, "\n"));
    out
}

/// The directories `worktree.sharedDirectories` holds in `text` right now – the
/// list Orca itself would use. Lenient by design: text that does not parse, has
/// no such key, or holds something other than a list of strings yields an empty
/// list, because the caller only asks "which directories does Orca share
/// already" and "none that we can tell" is the safe answer.
pub fn shared_directories(text: &str) -> Vec<String> {
    let body = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Ok(docs) = MarkedYaml::load_from_str(body) else {
        return Vec::new();
    };
    let [root] = docs.as_slice() else {
        return Vec::new();
    };
    entries(root)
        .and_then(|list| find(&list, WORKTREE))
        .and_then(|(_, worktree)| entries(worktree))
        .and_then(|list| find(&list, SHARED_DIRECTORIES))
        .and_then(|(_, shared)| match &shared.data {
            YamlData::Sequence(items) => Some(
                items
                    .iter()
                    .filter_map(|item| string_value(item).map(str::to_string))
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

/// The version named by a `%YAML` directive that is not `1.2`, if the text has
/// one. saphyr accepts `%YAML 1.1` silently and still parses with 1.2 rules,
/// while Orca's parser would switch to 1.1 semantics (`yes` becomes a boolean),
/// so the two could disagree about what the file says.
fn foreign_yaml_directive(body: &str) -> Option<String> {
    body.lines().find_map(|line| {
        let rest = line.strip_prefix("%YAML")?;
        let version = rest.split_whitespace().next().unwrap_or("");
        (version != "1.2").then(|| version.to_string())
    })
}

// ---------------------------------------------------------------------------
// Source text addressed by character offset.
// ---------------------------------------------------------------------------

/// The text being edited, held as characters because every position saphyr
/// reports is a character offset.
struct Source {
    chars: Vec<char>,
    /// The line ending the file uses, so inserted lines match it.
    eol: &'static str,
}

/// Replace characters `start..end` with `text`.
struct Edit {
    start: usize,
    end: usize,
    text: String,
}

impl Source {
    fn new(body: &str) -> Self {
        Self {
            chars: body.chars().collect(),
            eol: if body.contains("\r\n") { "\r\n" } else { "\n" },
        }
    }

    fn len(&self) -> usize {
        self.chars.len()
    }

    fn get(&self, idx: usize) -> Option<char> {
        self.chars.get(idx).copied()
    }

    /// Apply non-overlapping edits and return the resulting text.
    fn apply(&self, edits: &[Edit]) -> String {
        let mut ordered: Vec<&Edit> = edits.iter().collect();
        // Stable, so two insertions at one position keep the order they were
        // planned in (the `scripts` block before the `worktree` block).
        ordered.sort_by_key(|e| e.start);
        let mut out = String::new();
        let mut cursor = 0;
        for edit in ordered {
            let start = edit.start.clamp(cursor, self.len());
            out.extend(&self.chars[cursor..start]);
            out.push_str(&edit.text);
            cursor = edit.end.clamp(start, self.len());
        }
        out.extend(&self.chars[cursor..]);
        out
    }

    /// Index just past the first `\n` at or after `idx`, or the end of the text.
    fn next_line_start(&self, idx: usize) -> usize {
        let mut i = idx;
        while i < self.len() {
            if self.chars[i] == '\n' {
                return i + 1;
            }
            i += 1;
        }
        self.len()
    }

    /// Index of the first character of the line holding `idx`.
    fn line_start(&self, idx: usize) -> usize {
        let mut i = idx.min(self.len());
        while i > 0 && self.chars[i - 1] != '\n' {
            i -= 1;
        }
        i
    }

    /// Index of the line break ending the line that holds `idx` – the `\r` of a
    /// `\r\n` pair, so the result is where the line's own text stops.
    fn line_end(&self, idx: usize) -> usize {
        let mut i = idx.min(self.len());
        while i < self.len() && self.chars[i] != '\n' {
            i += 1;
        }
        if i > 0 && i <= self.len() && self.get(i) == Some('\n') && self.chars[i - 1] == '\r' {
            i -= 1;
        }
        i
    }

    /// Number of leading spaces on the line holding `idx`.
    fn line_indent(&self, idx: usize) -> usize {
        let start = self.line_start(idx);
        self.chars[start..]
            .iter()
            .take_while(|c| **c == ' ')
            .count()
    }

    /// Index just past the `:` that follows a key ending at `key_end`. `None`
    /// when something other than spaces sits between the two, which is the
    /// explicit `? key` / `: value` form this editor does not handle.
    fn after_colon(&self, key_end: usize) -> Option<usize> {
        let mut i = key_end;
        while matches!(self.get(i), Some(' ' | '\t')) {
            i += 1;
        }
        (self.get(i) == Some(':')).then_some(i + 1)
    }

    /// Index just past any node properties – an `&anchor`, a `!tag` – written
    /// at `idx`, on the same line. Returns `idx` itself when there are none, so
    /// a caller that splices text in at the result leaves properties where they
    /// were and otherwise writes exactly where it would have.
    fn skip_properties(&self, idx: usize) -> usize {
        let mut end = idx;
        let mut i = idx;
        loop {
            while matches!(self.get(i), Some(' ' | '\t')) {
                i += 1;
            }
            if !matches!(self.get(i), Some('&' | '!')) {
                return end;
            }
            while self.get(i).is_some_and(|c| !c.is_whitespace()) {
                i += 1;
            }
            end = i;
        }
    }

    /// Index of the first character at or after `idx` that is neither white
    /// space nor inside a comment – where a value written after `idx` starts.
    fn first_content(&self, idx: usize) -> Option<usize> {
        let mut i = idx;
        while let Some(c) = self.get(i) {
            match c {
                ' ' | '\t' | '\r' | '\n' => i += 1,
                // A `#` opens a comment only at the start of a line or after
                // white space, which is the only way this loop reaches one.
                '#' => i = self.line_end(i),
                _ => return Some(i),
            }
        }
        None
    }

    /// Walk back from `end` over white space, never below `floor`.
    fn trim_whitespace_back(&self, end: usize, floor: usize) -> usize {
        let mut i = end.min(self.len());
        while i > floor && self.chars[i - 1].is_whitespace() {
            i -= 1;
        }
        i
    }

    /// Walk back from `end` over blank lines AND comment-only lines, never below
    /// `floor`. What remains ends at the last line that holds real content,
    /// which is where a block collection stops; the comments after it belong to
    /// whatever comes next and must stay in the file.
    fn trim_trivia_back(&self, end: usize, floor: usize) -> usize {
        let mut i = end;
        loop {
            i = self.trim_whitespace_back(i, floor);
            if i <= floor {
                return floor;
            }
            let line = self.line_start(i - 1).max(floor);
            let first = self.chars[line..i].iter().find(|c| !c.is_whitespace());
            if first == Some(&'#') {
                i = line;
            } else {
                return i;
            }
        }
    }

    /// Index just past the closing quote of the double-quoted scalar opening at
    /// `start`.
    fn scan_double_quoted(&self, start: usize) -> Option<usize> {
        let mut i = start + 1;
        loop {
            match self.get(i)? {
                '\\' => i += 2,
                '"' => return Some(i + 1),
                _ => i += 1,
            }
        }
    }

    /// Index just past the closing quote of the single-quoted scalar opening at
    /// `start`. A doubled `''` is an escaped quote, not the end.
    fn scan_single_quoted(&self, start: usize) -> Option<usize> {
        let mut i = start + 1;
        loop {
            if self.get(i)? == '\'' {
                if self.get(i + 1) == Some('\'') {
                    i += 2;
                } else {
                    return Some(i + 1);
                }
            } else {
                i += 1;
            }
        }
    }

    /// Index just past the bracket closing the flow collection that opens at
    /// `start`, skipping quoted scalars and comments on the way.
    fn scan_flow(&self, start: usize) -> Option<usize> {
        let mut depth = 0usize;
        let mut i = start;
        loop {
            let c = self.get(i)?;
            // A quote or `#` only has its special meaning at the start of a
            // token; inside a plain scalar (`it's`) it is an ordinary character.
            let at_token_start = i == start
                || matches!(
                    self.chars[i - 1],
                    ' ' | '\t' | '\r' | '\n' | '[' | '{' | ',' | ':'
                );
            match c {
                '[' | '{' => {
                    depth += 1;
                    i += 1;
                }
                ']' | '}' => {
                    depth = depth.checked_sub(1)?;
                    i += 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                '"' if at_token_start => i = self.scan_double_quoted(i)?,
                '\'' if at_token_start => i = self.scan_single_quoted(i)?,
                '#' if at_token_start => i = self.line_end(i),
                _ => i += 1,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Reading the parsed document.
// ---------------------------------------------------------------------------

type Entry<'a, 'input> = (&'a MarkedYaml<'input>, &'a MarkedYaml<'input>);

/// The string a mapping key holds, when the key is a plain string scalar.
fn key_text<'a>(key: &'a MarkedYaml<'_>) -> Option<&'a str> {
    match &key.data {
        YamlData::Value(Scalar::String(s)) => Some(s.as_ref()),
        _ => None,
    }
}

/// The string a node holds, when it is a string scalar.
fn string_value<'a>(node: &'a MarkedYaml<'_>) -> Option<&'a str> {
    key_text(node)
}

/// The entries of a mapping node, in document order. A tagged mapping
/// (`!!map`, or a custom tag) is still a mapping, so the tag is looked through.
fn entries<'a, 'input>(node: &'a MarkedYaml<'input>) -> Option<Vec<Entry<'a, 'input>>> {
    match &node.data {
        YamlData::Mapping(map) => Some(map.iter().collect()),
        YamlData::Tagged(_, inner) => entries(inner),
        _ => None,
    }
}

fn find<'a, 'input>(list: &[Entry<'a, 'input>], name: &str) -> Option<Entry<'a, 'input>> {
    list.iter()
        .copied()
        .find(|(key, _)| key_text(key) == Some(name))
}

/// Whether a mapping carries a `<<` merge key. Orca's parser reads it as an
/// ordinary key, but it signals that the author expects keys to be pulled in
/// from elsewhere, so the mapping's real contents are not what the text shows.
fn has_merge_key(list: &[Entry<'_, '_>]) -> bool {
    list.iter().any(|(key, _)| key_text(key) == Some("<<"))
}

/// Whether `node` is a sequence holding exactly the strings in `expected`.
fn is_string_list(node: &MarkedYaml<'_>, expected: &[String]) -> bool {
    match &node.data {
        YamlData::Sequence(items) => {
            items.len() == expected.len()
                && items
                    .iter()
                    .zip(expected)
                    .all(|(item, want)| string_value(item) == Some(want.as_str()))
        }
        _ => false,
    }
}

/// Whether some line of `script` is the setup line, ignoring the white space
/// around it. This is the only test for "already present", so no write can ever
/// add the line twice.
pub fn has_setup_line(script: &str) -> bool {
    script.lines().any(|line| line.trim() == SETUP_LINE)
}

// ---------------------------------------------------------------------------
// Planning: where each managed key is, and what has to happen to it.
// ---------------------------------------------------------------------------

/// Where a managed child key (`setup`, `sharedDirectories`) stands in the file.
enum Slot {
    /// Nothing to do: the value already is what was intended.
    Keep,
    /// The parent key is absent from the top level; a whole block is appended
    /// at the end of the file.
    AppendBlock,
    /// The parent is a block mapping without the child; a new entry goes on the
    /// line after the parent's key, ahead of the existing children.
    InsertEntry {
        at: usize,
        child_indent: usize,
        unit: usize,
    },
    /// The child exists; characters `start..end` (everything after its colon up
    /// to the end of its value) are replaced.
    ReplaceValue {
        start: usize,
        end: usize,
        /// Column of the child key, from which nested lines are indented.
        key_indent: usize,
        unit: usize,
        extent: Extent,
    },
}

/// What [`Plan::value_end`] learned about an existing value besides its end.
#[derive(Default)]
struct Extent {
    /// Set when the old value is a block sequence: the index where the line of
    /// its first item starts, and the column its dashes sit at. A new list is
    /// written from that line on, at that column, so the list keeps its
    /// indentation style and anything between the key and the first item – a
    /// comment on the key line, comment lines above the items – stays put.
    block_items: Option<(usize, usize)>,
    /// Set when the old value is a block scalar whose header line (`setup: |`)
    /// carries a comment. The header is rewritten, so the comment is carried
    /// over to the new one.
    header_comment: Option<String>,
}

/// How the setup script is written. [`edit`] tries them in this order and
/// keeps the first whose result the re-parse check accepts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SetupStyle {
    /// A literal block scalar, with any trailing line breaks beyond the first
    /// written out as empty lines.
    Block,
    /// The same, without those empty lines. Right when the old value was
    /// itself a keep-chomped block (`|+`): the blank lines that followed it are
    /// still in the file and already count toward the script's ending.
    BlockUnpadded,
    /// A double-quoted scalar, which can carry any string.
    Quoted,
}

struct Plan<'p> {
    setup: Slot,
    shared: Slot,
    /// The value `scripts.setup` must have afterwards, or `None` when `scripts`
    /// is not to be touched at all.
    intended_setup: Option<String>,
    shared_dirs: &'p [String],
    /// Column of the top-level keys, and the indentation step the file uses.
    root_indent: usize,
    root_unit: usize,
}

impl<'p> Plan<'p> {
    fn build(
        src: &Source,
        root: Option<&MarkedYaml<'_>>,
        shared_dirs: &'p [String],
        ensure_setup: bool,
    ) -> Result<Self, String> {
        let root_entries = match root {
            None => Vec::new(),
            Some(node) => {
                let Some(list) = entries(node) else {
                    return Err("its top level is not a mapping".to_string());
                };
                if src.get(node.span.start.index()) == Some('{') {
                    return Err(
                        "its top level is a flow mapping (`{ … }`), which ss-magic does not edit"
                            .to_string(),
                    );
                }
                list
            }
        };
        let root_indent = root_entries
            .first()
            .map(|(key, _)| key.span.start.col())
            .unwrap_or(0);
        let root_unit = root_entries
            .iter()
            .find_map(|(key, value)| {
                let child = entries(value)?.first()?.0.span.start;
                (child.line() > key.span.start.line() && child.col() > key.span.start.col())
                    .then(|| child.col() - key.span.start.col())
            })
            .unwrap_or(DEFAULT_INDENT_UNIT);

        let shared = Self::locate(src, &root_entries, WORKTREE, SHARED_DIRECTORIES, |value| {
            Ok(is_string_list(value, shared_dirs))
        })?;

        let mut intended_setup = None;
        let setup = if ensure_setup {
            let slot = Self::locate(src, &root_entries, SCRIPTS, SETUP, |value| {
                match &value.data {
                    YamlData::Value(Scalar::String(script)) => {
                        if has_setup_line(script) {
                            intended_setup = Some(script.to_string());
                            Ok(true)
                        } else if script.trim().is_empty() {
                            Ok(false)
                        } else {
                            intended_setup = Some(format!("{SETUP_LINE}\n{script}"));
                            Ok(false)
                        }
                    }
                    YamlData::Value(Scalar::Null) => Ok(false),
                    _ => Err("`scripts.setup` is not a string".to_string()),
                }
            })?;
            if intended_setup.is_none() {
                intended_setup = Some(format!("{SETUP_LINE}\n"));
            }
            slot
        } else {
            Slot::Keep
        };

        if has_merge_key(&root_entries)
            && (matches!(setup, Slot::AppendBlock) || matches!(shared, Slot::AppendBlock))
        {
            return Err("its top level uses a merge key (`<<`)".to_string());
        }

        Ok(Self {
            setup,
            shared,
            intended_setup,
            shared_dirs,
            root_indent,
            root_unit,
        })
    }

    /// Find `parent.child` and decide what has to happen to it. `satisfied`
    /// looks at the child's current value: `Ok(true)` means it already is what
    /// was intended (nothing is checked further, so an unusual layout that needs
    /// no edit is no reason to refuse), `Ok(false)` means it must be replaced,
    /// and an `Err` is a refusal.
    fn locate(
        src: &Source,
        root_entries: &[Entry<'_, '_>],
        parent: &str,
        child: &str,
        mut satisfied: impl FnMut(&MarkedYaml<'_>) -> Result<bool, String>,
    ) -> Result<Slot, String> {
        let Some((parent_key, parent_value)) = find(root_entries, parent) else {
            return Ok(Slot::AppendBlock);
        };
        let children = entries(parent_value);
        if let Some((_, value)) = children.as_deref().and_then(|list| find(list, child)) {
            if satisfied(value)? {
                return Ok(Slot::Keep);
            }
        }

        // An edit is needed from here on, so the parent has to be a plain block
        // mapping. The layout is read from the source text: the parsed model
        // looks the same for a block mapping, a flow mapping and a resolved
        // alias.
        let after_parent = src
            .after_colon(parent_key.span.end.index())
            .ok_or_else(|| format!("`{parent}` is not written as a plain `{parent}:` key"))?;
        // Checked first: an empty value has no text of its own, so the "first
        // content after the colon" below would be whatever key comes next.
        if is_null(parent_value) {
            return Err(format!("`{parent}` is present but empty (null)"));
        }
        // An anchor or tag written after the colon is stepped over: it stays
        // where it is, and what counts is the node that follows it.
        let lead = src.first_content(src.skip_properties(after_parent));
        match lead.and_then(|idx| src.get(idx)) {
            Some('{') => {
                return Err(format!(
                    "`{parent}` is a flow mapping (`{{ … }}`), which ss-magic does not edit"
                ));
            }
            Some('*') => {
                return Err(format!("`{parent}` is an alias (`*…`) to another node"));
            }
            _ => {}
        }
        let Some(children) = children else {
            return Err(format!("`{parent}` is not a mapping"));
        };
        // For a block mapping the value starts exactly at its first key, on a
        // line of its own. Anything else is a layout this editor has no rule
        // for (an explicit `? key`, for one).
        let Some((first_child, _)) = children.first().copied() else {
            return Err(format!("`{parent}` is not a block mapping"));
        };
        if lead != Some(first_child.span.start.index())
            || first_child.span.start.line() == parent_key.span.start.line()
        {
            return Err(format!("`{parent}` is not a block mapping"));
        }
        if has_merge_key(&children) {
            return Err(format!("`{parent}` uses a merge key (`<<`)"));
        }

        let child_indent = first_child.span.start.col();
        let unit = child_indent
            .saturating_sub(parent_key.span.start.col())
            .max(1);

        let Some((key, value)) = find(&children, child) else {
            return Ok(Slot::InsertEntry {
                at: src.next_line_start(parent_key.span.end.index()),
                child_indent,
                unit,
            });
        };

        // The replaced text starts AFTER any anchor or tag on the value, so
        // those stay in the file and keep applying to the new value.
        let start = src
            .after_colon(key.span.end.index())
            .map(|after| src.skip_properties(after))
            .ok_or_else(|| format!("`{parent}.{child}` is not written as a plain `{child}:` key"))?;
        let (end, extent) = Self::value_end(src, start, value)
            .ok_or_else(|| format!("`{parent}.{child}` is an alias (`*…`) to another node"))?;
        Ok(Slot::ReplaceValue {
            start,
            end,
            key_indent: key.span.start.col(),
            unit,
            extent,
        })
    }

    /// Where the value written from `after_colon` on ends, plus what else is
    /// worth knowing about it ([`Extent`]). `after_colon` is the position just
    /// past the key's colon and any anchor or tag that follows it. `None` when
    /// the value is an alias, which is another node's content rather than text
    /// of its own to replace.
    fn value_end(
        src: &Source,
        after_colon: usize,
        value: &MarkedYaml<'_>,
    ) -> Option<(usize, Extent)> {
        // `key:` with nothing after it. The parser reports the empty value at
        // the key itself, and there is no text to replace – only to insert.
        if is_null(value) && value.span.is_empty() {
            return Some((after_colon, Extent::default()));
        }
        let lead = src.first_content(after_colon)?;
        let span_start = value.span.start.index();
        let span_end = value.span.end.index().min(src.len());
        let plain = |end: usize| Some((end, Extent::default()));
        match src.get(lead)? {
            '*' => None,
            '"' => plain(src.scan_double_quoted(lead)?),
            '\'' => plain(src.scan_single_quoted(lead)?),
            '[' | '{' => plain(src.scan_flow(lead)?),
            // A block scalar: the span starts on the first content line and runs
            // to the next less-indented text, so only white space is walked
            // back – a `#` line inside a literal script is content, not a
            // comment. The header itself is the floor.
            '|' | '>' => {
                let header: String = src.chars[lead..src.line_end(lead)].iter().collect();
                let header_comment = header
                    .find(" #")
                    .map(|at| header[at + 1..].trim_end().to_string());
                Some((
                    src.trim_whitespace_back(span_end, lead + 1),
                    Extent {
                        header_comment,
                        ..Extent::default()
                    },
                ))
            }
            _ => match &value.data {
                // A block collection's span runs to the next token, past blank
                // lines and comments that belong to whatever follows.
                YamlData::Sequence(_) => Some((
                    src.trim_trivia_back(span_end, lead),
                    Extent {
                        block_items: Some((
                            src.line_start(span_start),
                            src.line_indent(span_start),
                        )),
                        ..Extent::default()
                    },
                )),
                YamlData::Mapping(_) => plain(src.trim_trivia_back(span_end, lead)),
                // A plain scalar's span is exact.
                _ => plain(span_end.max(lead)),
            },
        }
    }

    fn is_noop(&self) -> bool {
        matches!(self.setup, Slot::Keep) && matches!(self.shared, Slot::Keep)
    }

    /// Whether the two [`SetupStyle`] candidates can differ at all – only when
    /// an existing script is being rewritten.
    fn setup_style_matters(&self) -> bool {
        !matches!(self.setup, Slot::Keep)
    }

    /// The edits for one candidate, or `None` when the setup value cannot be
    /// written in `style`.
    fn edits(&self, src: &Source, style: SetupStyle) -> Option<Vec<Edit>> {
        let eol = src.eol;
        let mut edits = Vec::new();
        let mut appended = String::new();

        match &self.setup {
            Slot::Keep => {}
            Slot::AppendBlock => {
                if style != SetupStyle::Block {
                    return None;
                }
                appended.push_str(&scripts_block(self.root_indent, self.root_unit, eol));
            }
            Slot::InsertEntry {
                at,
                child_indent,
                unit,
            } => {
                let value = self.intended_setup.as_deref()?;
                let written = setup_value(value, style, child_indent + unit, eol, None)?;
                edits.push(Edit {
                    start: *at,
                    end: *at,
                    text: format!("{}{SETUP}:{written}{eol}", " ".repeat(*child_indent)),
                });
            }
            Slot::ReplaceValue {
                start,
                end,
                key_indent,
                unit,
                extent,
            } => {
                let value = self.intended_setup.as_deref()?;
                // A comment after the old value on the same line would be read
                // as script text once a block scalar's last line ends there.
                // YAML allows a comment on the block header line, so for a block
                // candidate it moves up instead of being lost. An old block
                // header's own comment is carried over the same way.
                let line_end = src.line_end(*end);
                let tail: String = src.chars[*end..line_end].iter().collect();
                let tail = tail.trim();
                let (end, comment) = if let Some(comment) = &extent.header_comment {
                    (*end, Some(comment.clone()))
                } else if style != SetupStyle::Quoted && tail.starts_with('#') {
                    (line_end, Some(tail.to_string()))
                } else {
                    (*end, None)
                };
                let mut written = setup_value(
                    value,
                    style,
                    key_indent + unit,
                    eol,
                    comment.as_deref(),
                )?;
                if style == SetupStyle::Quoted {
                    if let Some(comment) = &extent.header_comment {
                        written.push(' ');
                        written.push_str(comment);
                    }
                }
                edits.push(Edit {
                    start: *start,
                    end,
                    text: written,
                });
            }
        }

        match &self.shared {
            Slot::Keep => {}
            Slot::AppendBlock => {
                appended.push_str(&worktree_block(
                    self.shared_dirs,
                    self.root_indent,
                    self.root_unit,
                    eol,
                ));
            }
            Slot::InsertEntry {
                at,
                child_indent,
                unit,
            } => edits.push(Edit {
                start: *at,
                end: *at,
                text: format!(
                    "{}{SHARED_DIRECTORIES}:{}{eol}",
                    " ".repeat(*child_indent),
                    sequence_value(self.shared_dirs, child_indent + unit, eol)
                ),
            }),
            Slot::ReplaceValue {
                start,
                end,
                key_indent,
                unit,
                extent,
            } => match extent.block_items {
                // The old list is a block sequence. Only its item lines are
                // rewritten, so a comment on the key line or above the first
                // item is not disturbed.
                Some((items_start, item_indent)) if !self.shared_dirs.is_empty() => {
                    let pad = " ".repeat(item_indent);
                    let items: Vec<String> = self
                        .shared_dirs
                        .iter()
                        .map(|dir| format!("{pad}- {}", scalar_item(dir)))
                        .collect();
                    edits.push(Edit {
                        start: items_start,
                        end: *end,
                        text: items.join(eol),
                    });
                }
                // Emptying a block sequence: the `[]` has to sit on the key
                // line, and the item lines go away whole.
                Some((items_start, _)) => {
                    edits.push(Edit {
                        start: *start,
                        end: *start,
                        text: " []".to_string(),
                    });
                    edits.push(Edit {
                        start: items_start,
                        end: src.next_line_start(*end),
                        text: String::new(),
                    });
                }
                None => edits.push(Edit {
                    start: *start,
                    end: *end,
                    text: sequence_value(self.shared_dirs, key_indent + unit, eol),
                }),
            },
        }

        if !appended.is_empty() {
            // A file whose last line has no line break must not have the new
            // block glued onto that line.
            let needs_break = src.len() > 0 && src.chars[src.len() - 1] != '\n';
            edits.push(Edit {
                start: src.len(),
                end: src.len(),
                text: format!("{}{appended}", if needs_break { eol } else { "" }),
            });
        }
        Some(edits)
    }
}

fn is_null(node: &MarkedYaml<'_>) -> bool {
    matches!(node.data, YamlData::Value(Scalar::Null))
}

// ---------------------------------------------------------------------------
// Writing YAML text.
// ---------------------------------------------------------------------------

/// A whole `scripts:` block holding only the setup line.
fn scripts_block(indent: usize, unit: usize, eol: &str) -> String {
    format!(
        "{pad}{SCRIPTS}:{eol}{child}{SETUP}: |{eol}{line}{SETUP_LINE}{eol}",
        pad = " ".repeat(indent),
        child = " ".repeat(indent + unit),
        line = " ".repeat(indent + 2 * unit),
    )
}

/// A whole `worktree:` block holding only the directory list.
fn worktree_block(dirs: &[String], indent: usize, unit: usize, eol: &str) -> String {
    format!(
        "{pad}{WORKTREE}:{eol}{child}{SHARED_DIRECTORIES}:{list}{eol}",
        pad = " ".repeat(indent),
        child = " ".repeat(indent + unit),
        list = sequence_value(dirs, indent + 2 * unit, eol),
    )
}

/// The text that follows `sharedDirectories:` – a block sequence with one item
/// per line at `item_indent`, or ` []` for an empty list (the only way to write
/// an empty sequence). No trailing line break: whatever followed the old value
/// follows the new one.
fn sequence_value(dirs: &[String], item_indent: usize, eol: &str) -> String {
    if dirs.is_empty() {
        return " []".to_string();
    }
    let pad = " ".repeat(item_indent);
    dirs.iter()
        .map(|dir| format!("{eol}{pad}- {}", scalar_item(dir)))
        .collect()
}

/// The text that follows `setup:` for `value` in the given style, or `None`
/// when a block scalar cannot carry the value.
fn setup_value(
    value: &str,
    style: SetupStyle,
    line_indent: usize,
    eol: &str,
    header_comment: Option<&str>,
) -> Option<String> {
    match style {
        SetupStyle::Quoted => Some(format!(" {}", double_quoted(value))),
        SetupStyle::Block | SetupStyle::BlockUnpadded => {
            let (header, mut lines) = block_scalar(value)?;
            if style == SetupStyle::BlockUnpadded {
                let padded = lines.len();
                while lines.last().is_some_and(|line| line.is_empty()) {
                    lines.pop();
                }
                // Identical to the padded candidate, which was already tried.
                if lines.len() == padded {
                    return None;
                }
            }
            let pad = " ".repeat(line_indent);
            let mut out = format!(" {header}");
            if let Some(comment) = header_comment {
                out.push(' ');
                out.push_str(comment);
            }
            for line in lines {
                out.push_str(eol);
                if !line.is_empty() {
                    out.push_str(&pad);
                    out.push_str(line);
                }
            }
            Some(out)
        }
    }
}

/// Split `value` into a literal block scalar header and its content lines, or
/// `None` when the string cannot be written that way.
///
/// The chomping indicator is chosen from the string's own trailing line breaks,
/// so the script keeps exactly the ending it had: none is `|-`, one is `|`, more
/// is `|+` with the extra breaks written as empty lines.
///
/// Refused here, and written double-quoted instead: an empty script, any control
/// character (a block scalar has no escapes), a line starting with a tab (YAML
/// parsers disagree about tabs next to indentation), and a first line starting
/// with a space (it would need an explicit indentation indicator).
fn block_scalar(value: &str) -> Option<(String, Vec<&str>)> {
    let content = value.trim_end_matches('\n');
    let trailing = value.len() - content.len();
    if content.is_empty() || content.starts_with(' ') {
        return None;
    }
    let representable = content.split('\n').all(|line| {
        !line.starts_with('\t')
            && line.chars().all(|c| {
                c == '\t'
                    || !(c.is_control()
                        || matches!(c, '\u{feff}' | '\u{2028}' | '\u{2029}'))
            })
    });
    if !representable {
        return None;
    }
    let chomp = match trailing {
        0 => "-",
        1 => "",
        _ => "+",
    };
    let mut lines: Vec<&str> = content.split('\n').collect();
    lines.extend(std::iter::repeat_n("", trailing.saturating_sub(1)));
    Some((format!("|{chomp}"), lines))
}

/// `value` as a YAML double-quoted scalar, which can carry any string.
fn double_quoted(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            c if (c as u32) < 0x100 && c.is_control() => {
                out.push_str(&format!("\\x{:02X}", c as u32));
            }
            '\u{feff}' | '\u{2028}' | '\u{2029}' => {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// One sequence item: written plain when that is certain to be read back as the
/// same string, double-quoted otherwise.
///
/// "Certain" is deliberately narrow. The text must start with a letter, `_`,
/// `.` or `/`, hold only letters, digits and `_ - . / @ +`, not be a
/// leading-dot number, not be a word some YAML version reads as a boolean or
/// null, and be reported as a string by saphyr itself. Ordinary directory paths (`node_modules`, `apps/web/.next/cache`)
/// pass; anything unusual costs a pair of quotes and nothing else.
fn scalar_item(text: &str) -> String {
    const WORDS: [&str; 11] = [
        "null", "true", "false", "yes", "no", "on", "off", "y", "n", ".inf", ".nan",
    ];
    let first_ok = text
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '_' | '.' | '/'));
    let charset_ok = text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '@' | '+'));
    // The first-character rule already keeps digits and signs out, so the only
    // number shape left is a leading-dot float such as `.5` or `.5e3`.
    let numeric_looking = text
        .strip_prefix('.')
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()));
    let reserved = WORDS.contains(&text.to_ascii_lowercase().as_str());
    let parses_as_string = matches!(
        Scalar::parse_from_cow(text.into()),
        Scalar::String(_)
    );
    if first_ok && charset_ok && !numeric_looking && !reserved && parses_as_string {
        text.to_string()
    } else {
        double_quoted(text)
    }
}

// ---------------------------------------------------------------------------
// The re-parse check.
// ---------------------------------------------------------------------------

/// Confirm that `candidate` says exactly what was intended and nothing else
/// changed: it parses as ONE document with a block-mapping top level,
/// `worktree.sharedDirectories` is the given list, `scripts.setup` is
/// `intended_setup` (or `scripts` is wholly unchanged when that is `None`), and
/// every other key – at the top level and beside the two managed keys – has the
/// same value, in the same order, as in `before`.
fn verify(
    before: Option<&MarkedYaml<'_>>,
    candidate: &str,
    intended_setup: Option<&str>,
    shared_dirs: &[String],
) -> Result<(), String> {
    let docs = MarkedYaml::load_from_str(candidate).map_err(|err| err.to_string())?;
    let [after] = docs.as_slice() else {
        return Err(format!("{} documents", docs.len()));
    };
    let after_entries = entries(after).ok_or("top level is not a mapping")?;
    if candidate.chars().nth(after.span.start.index()) == Some('{') {
        return Err("top level became a flow mapping".to_string());
    }
    let before_entries = before.and_then(entries).unwrap_or_default();

    let managed = |name: &str| name == SCRIPTS || name == WORKTREE;
    same_entries(
        &others(&before_entries, managed),
        &others(&after_entries, managed),
    )?;

    // `worktree`: the list is exactly the intended one, siblings untouched.
    let (_, worktree) = find(&after_entries, WORKTREE).ok_or("no `worktree` key")?;
    let worktree_entries = entries(worktree).ok_or("`worktree` is not a mapping")?;
    let (_, shared) =
        find(&worktree_entries, SHARED_DIRECTORIES).ok_or("no `sharedDirectories` key")?;
    if !is_string_list(shared, shared_dirs) {
        return Err("`sharedDirectories` is not the intended list".to_string());
    }
    let worktree_before = find(&before_entries, WORKTREE)
        .and_then(|(_, value)| entries(value))
        .unwrap_or_default();
    same_entries(
        &others(&worktree_before, |name| name == SHARED_DIRECTORIES),
        &others(&worktree_entries, |name| name == SHARED_DIRECTORIES),
    )?;

    // `scripts`: either wholly unchanged, or `setup` is the intended script and
    // its siblings are untouched.
    let scripts_before = find(&before_entries, SCRIPTS);
    let scripts_after = find(&after_entries, SCRIPTS);
    match intended_setup {
        None => match (scripts_before, scripts_after) {
            (None, None) => {}
            (Some((_, a)), Some((_, b))) if a == b => {}
            _ => return Err("`scripts` changed".to_string()),
        },
        Some(intended) => {
            let (_, scripts) = scripts_after.ok_or("no `scripts` key")?;
            let scripts_entries = entries(scripts).ok_or("`scripts` is not a mapping")?;
            let (_, setup) = find(&scripts_entries, SETUP).ok_or("no `setup` key")?;
            if string_value(setup) != Some(intended) {
                return Err("`scripts.setup` is not the intended script".to_string());
            }
            let before_list = scripts_before
                .and_then(|(_, value)| entries(value))
                .unwrap_or_default();
            same_entries(
                &others(&before_list, |name| name == SETUP),
                &others(&scripts_entries, |name| name == SETUP),
            )?;
        }
    }
    Ok(())
}

/// The entries whose key is not one of the managed names.
fn others<'a, 'input>(
    list: &[Entry<'a, 'input>],
    managed: impl Fn(&str) -> bool,
) -> Vec<Entry<'a, 'input>> {
    list.iter()
        .copied()
        .filter(|(key, _)| !key_text(key).is_some_and(&managed))
        .collect()
}

/// Two entry lists hold the same keys with the same values in the same order.
/// Node equality ignores source positions, so this compares meaning only.
fn same_entries(before: &[Entry<'_, '_>], after: &[Entry<'_, '_>]) -> Result<(), String> {
    if before.len() != after.len() {
        return Err("a key was added or lost".to_string());
    }
    for ((key_before, value_before), (key_after, value_after)) in before.iter().zip(after) {
        if key_before != key_after || value_before != value_after {
            return Err("an unmanaged key changed".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
