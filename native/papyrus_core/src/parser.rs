//! Hand-written recursive-descent Papyrus parser.
//!
//! Replaces `py_creation_lib/python/creation_lib/papyrus_lsp/parser.py` (Lark Earley + transformer). The grammar
//! being implemented is documented in
//! `plugins/papyrus_lsp_plugin/plugins/papyrus_lsp/grammar.lark`. Parity tests
//! live in `tests/test_parser.py` under that plugin.
//!
//! Negative-number tokens are not pre-classified by the lexer (see lexer.rs);
//! unary minus is handled here in `parse_unary`. Cast `as` and type-check `is`
//! are both consumed at the `cast_expr` level, but only `as` produces a node —
//! matching the Python transformer's behavior at `parser.py:603-620`.

use crate::ast::*;
use crate::lexer::{DocComment, Pos as LexPos, Token, TokenKind, tokenize_with_docs};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ParseError {
    pub line: u32,
    pub col: u32,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct ParseResult {
    pub ast: Option<ScriptNode>,
    pub errors: Vec<ParseError>,
    /// The script-level `{ ... }` doc comment (on the `Scriptname` line). Kept
    /// out of `ScriptNode` (ast.rs) and threaded to codegen separately.
    pub script_docstring: String,
    /// `Group ... EndGroup` declarations in source order. Kept out of
    /// `ScriptNode` (ast.rs) and threaded to codegen for the FO4 debug
    /// property-group table.
    pub property_groups: Vec<ParsedGroup>,
    /// Names of same-script `Struct` declarations. The parser otherwise discards
    /// struct bodies; codegen needs the names to qualify struct-typed fields as
    /// `script#struct`.
    pub struct_names: Vec<String>,
}

/// A `Group NAME flags { doc } ... EndGroup` declaration, captured for the
/// FO4 debug property-group table (its members stay in the flat property list).
#[derive(Debug, Clone, Default)]
pub struct ParsedGroup {
    pub name: String,
    pub docstring: String,
    pub flags: Vec<String>,
    pub prop_names: Vec<String>,
}

fn span(start: LexPos, end: LexPos) -> Pos {
    Pos {
        line: start.line,
        col: start.col,
        end_line: end.line,
        end_col: end.col,
    }
}

/// Public entry point. Tokenizes via `lexer::tokenize` (which preprocesses
/// CRLF, line continuations, and doc/block comments) and runs the parser.
pub fn parse_script(src: &str) -> ParseResult {
    if src.trim().is_empty() {
        return ParseResult {
            ast: None,
            errors: vec![ParseError {
                line: 1,
                col: 0,
                message: "Empty script".into(),
            }],
            ..Default::default()
        };
    }

    let (tokens, docs) = match tokenize_with_docs(src) {
        Ok(t) => t,
        Err(e) => {
            return ParseResult {
                ast: None,
                errors: vec![ParseError {
                    line: e.pos.line,
                    col: e.pos.col,
                    message: e.message,
                }],
                ..Default::default()
            };
        }
    };

    let mut p = Parser::new(tokens, docs);
    let ast = p.parse_full_script();
    ParseResult {
        ast,
        script_docstring: std::mem::take(&mut p.script_docstring),
        property_groups: std::mem::take(&mut p.property_groups),
        struct_names: std::mem::take(&mut p.struct_names),
        errors: p.errors,
    }
}

// ---------------------------------------------------------------------------
// Parser state
// ---------------------------------------------------------------------------

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    errors: Vec<ParseError>,
    /// Captured `{ ... }` doc comments, sorted by line; `take_docstring` claims
    /// one for a declaration and marks it consumed (line set to 0).
    docs: Vec<DocComment>,
    /// The script-level doc comment, claimed in `parse_full_script`.
    script_docstring: String,
    /// `Group` declarations captured in source order (see `ParsedGroup`).
    property_groups: Vec<ParsedGroup>,
    /// Names of `Struct` declarations (bodies discarded; names kept for codegen).
    struct_names: Vec<String>,
}

impl Parser {
    fn new(tokens: Vec<Token>, mut docs: Vec<DocComment>) -> Self {
        docs.sort_by_key(|d| d.line);
        Self {
            tokens,
            pos: 0,
            errors: Vec::new(),
            docs,
            script_docstring: String::new(),
            property_groups: Vec::new(),
            struct_names: Vec::new(),
        }
    }

    /// Claim the doc comment attached to a declaration starting on `start_line`
    /// (Papyrus puts it on that line or the next), returning its text or "".
    fn take_docstring(&mut self, start_line: u32) -> String {
        if let Some(d) = self
            .docs
            .iter_mut()
            .find(|d| d.line == start_line || d.line == start_line + 1)
        {
            d.line = 0; // consumed
            std::mem::take(&mut d.text)
        } else {
            String::new()
        }
    }

    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn peek_at(&self, n: usize) -> &TokenKind {
        match self.tokens.get(self.pos + n) {
            Some(t) => &t.kind,
            None => &TokenKind::Eof,
        }
    }

    fn cur(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn cur_pos(&self) -> LexPos {
        self.cur().start
    }

    fn last_end(&self) -> LexPos {
        if self.pos == 0 {
            self.tokens[0].end
        } else {
            self.tokens[self.pos - 1].end
        }
    }

    fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if !matches!(t.kind, TokenKind::Eof) {
            self.pos += 1;
        }
        t
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), TokenKind::Eof)
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), TokenKind::Newline) {
            self.pos += 1;
        }
    }

    fn eat_newline(&mut self) {
        if matches!(self.peek(), TokenKind::Newline) {
            self.pos += 1;
        }
    }

    fn check(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(self.peek()) == std::mem::discriminant(kind)
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.check(kind) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: &TokenKind, what: &str) -> bool {
        if self.eat(kind) {
            true
        } else {
            let pos = self.cur_pos();
            self.error_at(pos, format!("expected {what}, got {:?}", self.peek()));
            false
        }
    }

    fn error_at(&mut self, pos: LexPos, message: impl Into<String>) {
        self.errors.push(ParseError {
            line: pos.line,
            col: pos.col,
            message: message.into(),
        });
    }

    /// Skip to the next newline or top-level keyword. Used after a recoverable
    /// parse failure to stop the cascade.
    fn sync_to_top_level(&mut self) {
        while !self.at_eof() {
            match self.peek() {
                TokenKind::Newline => {
                    self.pos += 1;
                    return;
                }
                TokenKind::KwScriptname
                | TokenKind::KwImport
                | TokenKind::KwFunction
                | TokenKind::KwEvent
                | TokenKind::KwState
                | TokenKind::KwAuto
                | TokenKind::KwStruct
                | TokenKind::KwGroup
                | TokenKind::KwCustomEvent
                | TokenKind::KwEndState
                | TokenKind::KwEndProperty
                | TokenKind::KwEndFunction
                | TokenKind::KwEndEvent
                | TokenKind::KwEndStruct
                | TokenKind::KwEndGroup => return,
                _ => self.pos += 1,
            }
        }
    }

    // -----------------------------------------------------------------------
    // Top level
    // -----------------------------------------------------------------------

    fn parse_full_script(&mut self) -> Option<ScriptNode> {
        self.skip_newlines();
        let header = self.parse_script_header()?;
        self.script_docstring = self.take_docstring(header.pos.line);
        let mut script = ScriptNode {
            name: header.name,
            parent: header.parent,
            flags: header.flags,
            pos: header.pos,
            ..Default::default()
        };
        self.skip_newlines();
        while !self.at_eof() {
            match self.parse_top_level_item() {
                Some(TopLevel::Import(i)) => script.imports.push(i),
                Some(TopLevel::Property(props)) => script.properties.extend(props),
                Some(TopLevel::Variable(v)) => script.variables.push(v),
                Some(TopLevel::Function(f)) => script.functions.push(f),
                Some(TopLevel::Event(e)) => script.events.push(e),
                Some(TopLevel::State(s)) => script.states.push(s),
                Some(TopLevel::Struct(s)) => script.structs.push(s),
                Some(TopLevel::Discard) => {}
                None => {
                    if !self.at_eof() {
                        let p = self.cur_pos();
                        self.error_at(p, format!("unexpected token {:?}", self.peek()));
                        self.sync_to_top_level();
                    }
                }
            }
            self.skip_newlines();
        }
        Some(script)
    }

    fn parse_script_header(&mut self) -> Option<ScriptHeader> {
        let start = self.cur_pos();
        if !self.eat(&TokenKind::KwScriptname) {
            self.error_at(start, "expected `Scriptname` declaration");
            return None;
        }
        let name = self.parse_script_ident();
        let parent = if self.eat(&TokenKind::KwExtends) {
            Some(self.parse_script_ident())
        } else {
            None
        };
        let mut flags = Vec::new();
        loop {
            if let Some(flag) = match_script_flag(self.peek()) {
                self.pos += 1;
                flags.push(flag);
            } else {
                break;
            }
        }
        let end = self.last_end();
        Some(ScriptHeader {
            name,
            parent,
            flags,
            pos: span(start, end),
        })
    }

    /// Script identifier with optional `Namespace:Sub:...:Name`.
    fn parse_script_ident(&mut self) -> String {
        let mut parts = Vec::new();
        if let TokenKind::Name(n) = self.peek() {
            parts.push(n.clone());
            self.pos += 1;
        } else {
            let p = self.cur_pos();
            self.error_at(p, "expected identifier");
            return "Unknown".into();
        }
        while matches!(self.peek(), TokenKind::Colon) {
            self.pos += 1;
            if let TokenKind::Name(n) = self.peek() {
                parts.push(n.clone());
                self.pos += 1;
            } else {
                let p = self.cur_pos();
                self.error_at(p, "expected identifier after `:`");
                break;
            }
        }
        parts.join(":")
    }

    /// type_ref: type_name ("[]" | "[]"-via-LBRACKET-RBRACKET)?
    fn parse_type_ref(&mut self) -> String {
        let base = self.parse_type_name();
        if matches!(self.peek(), TokenKind::LBracket)
            && matches!(self.peek_at(1), TokenKind::RBracket)
        {
            self.pos += 2;
            return format!("{base}[]");
        }
        base
    }

    /// type_name: NAME (":" NAME)* | VAR_KW
    ///
    /// FO76 namespaced types can have multiple segments
    /// (e.g. `quests:_default:progressbar:masterscript`), so consume every
    /// `:NAME` segment, not just the first.
    fn parse_type_name(&mut self) -> String {
        if matches!(self.peek(), TokenKind::KwVar) {
            self.pos += 1;
            return "Var".into();
        }
        let mut parts = Vec::new();
        if let TokenKind::Name(n) = self.peek() {
            parts.push(n.clone());
            self.pos += 1;
        } else {
            let p = self.cur_pos();
            self.error_at(p, "expected type name");
            return "Unknown".into();
        }
        while matches!(self.peek(), TokenKind::Colon)
            && matches!(self.peek_at(1), TokenKind::Name(_))
        {
            self.pos += 1;
            if let TokenKind::Name(n) = self.peek() {
                parts.push(n.clone());
                self.pos += 1;
            }
        }
        parts.join(":")
    }

    fn parse_top_level_item(&mut self) -> Option<TopLevel> {
        match self.peek() {
            TokenKind::KwImport => Some(TopLevel::Import(self.parse_import())),
            TokenKind::KwState | TokenKind::KwAuto => Some(TopLevel::State(self.parse_state())),
            TokenKind::KwStruct => Some(TopLevel::Struct(self.parse_struct())),
            TokenKind::KwGroup => {
                let props = self.parse_group();
                Some(TopLevel::Property(props))
            }
            TokenKind::KwCustomEvent => {
                self.parse_custom_event_discard();
                Some(TopLevel::Discard)
            }
            TokenKind::KwFunction => {
                let f = self.parse_function(None, self.cur_pos());
                Some(TopLevel::Function(f))
            }
            TokenKind::KwEvent => Some(TopLevel::Event(self.parse_event())),
            TokenKind::Name(_) | TokenKind::KwVar => {
                // Could be: type Property NAME ...
                //           type NAME = expr   (variable)
                //           type NAME var_flag*  (variable, no init)
                //           [type] Function NAME ...
                let start = self.cur_pos();
                let ty = self.parse_type_ref();
                match self.peek() {
                    TokenKind::KwProperty => {
                        let p = self.parse_property(ty, start);
                        Some(TopLevel::Property(vec![p]))
                    }
                    TokenKind::KwFunction => {
                        let f = self.parse_function(Some(ty), start);
                        Some(TopLevel::Function(f))
                    }
                    TokenKind::Name(_) => {
                        let v = self.parse_variable(ty, start);
                        Some(TopLevel::Variable(v))
                    }
                    _ => {
                        let p = self.cur_pos();
                        self.error_at(p, format!("unexpected token {:?} after type", self.peek()));
                        self.sync_to_top_level();
                        Some(TopLevel::Discard)
                    }
                }
            }
            TokenKind::Eof => None,
            _ => None,
        }
    }

    fn parse_import(&mut self) -> ImportNode {
        let start = self.cur_pos();
        self.expect(&TokenKind::KwImport, "`Import`");
        let name = match self.peek() {
            TokenKind::Name(n) => {
                let s = n.clone();
                self.pos += 1;
                s
            }
            _ => {
                let p = self.cur_pos();
                self.error_at(p, "expected script name after `Import`");
                "Unknown".into()
            }
        };
        let end = self.last_end();
        ImportNode {
            script_name: name,
            pos: span(start, end),
        }
    }

    /// STRUCT NAME NL (TYPE NAME (= expr)? flag*)* ENDSTRUCT
    fn parse_struct(&mut self) -> StructDef {
        let start = self.cur_pos();
        self.pos += 1; // struct
        let name = match self.peek() {
            TokenKind::Name(n) => {
                let s = n.clone();
                self.pos += 1;
                s
            }
            _ => {
                let p = self.cur_pos();
                self.error_at(p, "expected struct name");
                "Unknown".into()
            }
        };
        self.struct_names.push(name.clone());
        self.skip_newlines();
        let mut members = Vec::new();
        while !self.at_eof() && !matches!(self.peek(), TokenKind::KwEndStruct) {
            let mstart = self.cur_pos();
            let ty = self.parse_type_ref();
            let mname = match self.peek() {
                TokenKind::Name(n) => {
                    let s = n.clone();
                    self.pos += 1;
                    s
                }
                _ => {
                    let p = self.cur_pos();
                    self.error_at(p, "expected struct member name");
                    while !self.at_eof() && !matches!(self.peek(), TokenKind::KwEndStruct) {
                        self.pos += 1;
                    }
                    break;
                }
            };
            let mut value = None;
            if self.eat(&TokenKind::Assign) {
                value = Some(self.parse_expr());
            }
            let mut flags = Vec::new();
            while let Some(flag) = match_struct_member_flag(self.peek()) {
                self.pos += 1;
                flags.push(flag);
            }
            members.push(StructMemberDef {
                name: mname,
                ty,
                value,
                flags,
                pos: span(mstart, self.last_end()),
            });
            self.skip_newlines();
        }
        self.eat(&TokenKind::KwEndStruct);
        StructDef {
            name,
            members,
            pos: span(start, self.last_end()),
        }
    }

    fn parse_custom_event_discard(&mut self) {
        self.pos += 1; // CustomEvent
        if matches!(self.peek(), TokenKind::Name(_)) {
            self.pos += 1;
        }
    }

    fn parse_group(&mut self) -> Vec<PropertyDef> {
        // GROUP NAME group_flag* { doc } NL (property_def NL)* ENDGROUP
        let group_line = self.cur_pos().line;
        self.pos += 1; // group
        let name = match self.peek() {
            TokenKind::Name(n) => {
                let n = n.clone();
                self.pos += 1;
                n
            }
            _ => String::new(),
        };
        let mut flags = Vec::new();
        loop {
            match self.peek() {
                TokenKind::KwCollapsed => flags.push("collapsed".to_string()),
                TokenKind::KwCollapsedOnRef => flags.push("collapsedonref".to_string()),
                TokenKind::KwCollapsedOnBase => flags.push("collapsedonbase".to_string()),
                _ => break,
            }
            self.pos += 1;
        }
        let docstring = self.take_docstring(group_line);
        self.skip_newlines();
        let mut props = Vec::new();
        while !self.at_eof() && !matches!(self.peek(), TokenKind::KwEndGroup) {
            // property: type Property NAME ...
            let start = self.cur_pos();
            if matches!(self.peek(), TokenKind::Name(_) | TokenKind::KwVar) {
                let ty = self.parse_type_ref();
                if matches!(self.peek(), TokenKind::KwProperty) {
                    props.push(self.parse_property(ty, start));
                } else {
                    self.sync_to_top_level();
                }
            } else {
                self.sync_to_top_level();
            }
            self.skip_newlines();
        }
        self.eat(&TokenKind::KwEndGroup);
        self.property_groups.push(ParsedGroup {
            name,
            docstring,
            flags,
            prop_names: props.iter().map(|p| p.name.clone()).collect(),
        });
        props
    }

    fn parse_property(&mut self, ty: String, type_start: LexPos) -> PropertyDef {
        // already at PROPERTY_KW
        self.pos += 1; // property
        let name = match self.peek() {
            TokenKind::Name(n) => {
                let s = n.clone();
                self.pos += 1;
                s
            }
            _ => {
                let p = self.cur_pos();
                self.error_at(p, "expected property name");
                "Unknown".into()
            }
        };
        let mut default = None;
        if self.eat(&TokenKind::Assign) {
            default = Some(self.parse_expr());
        }
        let mut flags = Vec::new();
        while let Some(flag) = match_prop_flag(self.peek()) {
            self.pos += 1;
            flags.push(flag);
        }
        let mut getter = None;
        let mut setter = None;
        // Decide short vs long form. A property with `Auto` or `AutoReadOnly`
        // is auto-implemented and never has a body — anything that follows
        // belongs to a sibling. Without those flags, peek past newlines: if we
        // see `EndProperty` or a function declaration, it's the long form.
        let has_auto = flags.iter().any(|f| f == "Auto" || f == "AutoReadOnly");
        let long_form = if has_auto {
            false
        } else {
            let saved = self.pos;
            self.skip_newlines();
            let is_long = match self.peek() {
                TokenKind::KwEndProperty | TokenKind::KwFunction => true,
                TokenKind::Name(_) | TokenKind::KwVar => self.lookahead_starts_function(),
                _ => false,
            };
            self.pos = saved;
            is_long
        };
        if !long_form {
            // short form — leave the cursor where it is for the top-level loop.
        } else {
            self.skip_newlines();
            while !self.at_eof() && !matches!(self.peek(), TokenKind::KwEndProperty) {
                let f_start = self.cur_pos();
                let return_type = if matches!(self.peek(), TokenKind::Name(_) | TokenKind::KwVar) {
                    Some(self.parse_type_ref())
                } else {
                    None
                };
                if matches!(self.peek(), TokenKind::KwFunction) {
                    let f = self.parse_function(return_type, f_start);
                    let lname = f.name.to_ascii_lowercase();
                    if lname.starts_with("get") {
                        getter = Some(f);
                    } else if lname.starts_with("set") {
                        setter = Some(f);
                    }
                } else {
                    // Recover.
                    let p = self.cur_pos();
                    self.error_at(p, "expected `Function` inside property body");
                    self.sync_to_top_level();
                }
                self.skip_newlines();
            }
            self.eat(&TokenKind::KwEndProperty);
        }
        let end = self.last_end();
        let docstring = self.take_docstring(type_start.line);
        PropertyDef {
            name,
            ty,
            flags,
            default,
            getter,
            setter,
            docstring,
            pos: span(type_start, end),
            ..Default::default()
        }
    }

    /// Peek a few tokens ahead to see if the current type-like prefix is the
    /// start of a function declaration (used to disambiguate property body
    /// items vs. sibling properties).
    fn lookahead_starts_function(&self) -> bool {
        // Walk type_ref tokens (NAME, optional `:` NAME, optional `[]`) and
        // check if the next is `Function`.
        let mut i = 0usize;
        // type_name
        match self.peek_at(i) {
            TokenKind::Name(_) | TokenKind::KwVar => i += 1,
            _ => return false,
        }
        if matches!(self.peek_at(i), TokenKind::Colon)
            && matches!(self.peek_at(i + 1), TokenKind::Name(_))
        {
            i += 2;
        }
        if matches!(self.peek_at(i), TokenKind::LBracket)
            && matches!(self.peek_at(i + 1), TokenKind::RBracket)
        {
            i += 2;
        }
        matches!(self.peek_at(i), TokenKind::KwFunction)
    }

    fn parse_variable(&mut self, ty: String, start: LexPos) -> VariableDef {
        // already past type_ref; current token must be NAME.
        let name = match self.peek() {
            TokenKind::Name(n) => {
                let s = n.clone();
                self.pos += 1;
                s
            }
            _ => {
                let p = self.cur_pos();
                self.error_at(p, "expected variable name");
                "Unknown".into()
            }
        };
        let mut value = None;
        if self.eat(&TokenKind::Assign) {
            value = Some(self.parse_expr());
        }
        let mut flags = Vec::new();
        while let Some(flag) = match_var_flag(self.peek()) {
            self.pos += 1;
            flags.push(flag);
        }
        let end = self.last_end();
        VariableDef {
            name,
            ty,
            value,
            flags,
            pos: span(start, end),
        }
    }

    fn parse_function(&mut self, return_type: Option<String>, start: LexPos) -> FunctionDef {
        // current is FUNCTION_KW
        self.expect(&TokenKind::KwFunction, "`Function`");
        let name = match self.peek() {
            TokenKind::Name(n) => {
                let s = n.clone();
                self.pos += 1;
                s
            }
            _ => {
                let p = self.cur_pos();
                self.error_at(p, "expected function name");
                "Unknown".into()
            }
        };
        self.expect(&TokenKind::LParen, "`(`");
        let params = self.parse_param_list();
        self.expect(&TokenKind::RParen, "`)`");
        let mut is_native = false;
        let mut is_global = false;
        let mut is_beta_only = false;
        loop {
            match self.peek() {
                TokenKind::KwNative => {
                    is_native = true;
                    self.pos += 1;
                }
                TokenKind::KwGlobal => {
                    is_global = true;
                    self.pos += 1;
                }
                TokenKind::KwBetaOnly => {
                    is_beta_only = true;
                    self.pos += 1;
                }
                TokenKind::KwDebugOnly => {
                    self.pos += 1;
                }
                _ => break,
            }
        }
        // Body is present iff not native (Papyrus rule). If we see EndFunction
        // immediately, treat as empty body.
        let body = if is_native {
            Vec::new()
        } else {
            self.skip_newlines();
            let body = self.parse_block_until(&[TokenKind::KwEndFunction]);
            self.eat(&TokenKind::KwEndFunction);
            body
        };
        let end = self.last_end();
        let docstring = self.take_docstring(start.line);
        FunctionDef {
            name,
            return_type: return_type.unwrap_or_else(|| "None".into()),
            params,
            is_native,
            is_global,
            is_beta_only,
            docstring,
            body,
            pos: span(start, end),
        }
    }

    fn parse_event(&mut self) -> EventDef {
        let start = self.cur_pos();
        self.expect(&TokenKind::KwEvent, "`Event`");
        // event_name: NAME ("." NAME)?
        let mut name = String::new();
        if let TokenKind::Name(n) = self.peek() {
            name.push_str(n);
            self.pos += 1;
        }
        if matches!(self.peek(), TokenKind::Dot) && matches!(self.peek_at(1), TokenKind::Name(_)) {
            self.pos += 1;
            if let TokenKind::Name(n) = self.peek() {
                name.push('.');
                name.push_str(n);
                self.pos += 1;
            }
        }
        self.expect(&TokenKind::LParen, "`(`");
        let params = self.parse_param_list();
        self.expect(&TokenKind::RParen, "`)`");
        let mut is_native = false;
        loop {
            match self.peek() {
                TokenKind::KwNative => {
                    is_native = true;
                    self.pos += 1;
                }
                TokenKind::KwBetaOnly | TokenKind::KwDebugOnly => self.pos += 1,
                _ => break,
            }
        }
        let body = if is_native {
            Vec::new()
        } else {
            self.skip_newlines();
            let b = self.parse_block_until(&[TokenKind::KwEndEvent]);
            self.eat(&TokenKind::KwEndEvent);
            b
        };
        let end = self.last_end();
        let docstring = self.take_docstring(start.line);
        EventDef {
            name,
            params,
            is_native,
            docstring,
            body,
            pos: span(start, end),
        }
    }

    fn parse_state(&mut self) -> StateDef {
        let start = self.cur_pos();
        let is_auto = self.eat(&TokenKind::KwAuto);
        self.expect(&TokenKind::KwState, "`State`");
        let name = match self.peek() {
            TokenKind::Name(n) => {
                let s = n.clone();
                self.pos += 1;
                s
            }
            _ => {
                let p = self.cur_pos();
                self.error_at(p, "expected state name");
                "Unknown".into()
            }
        };
        self.skip_newlines();
        let mut functions = Vec::new();
        let mut events = Vec::new();
        while !self.at_eof() && !matches!(self.peek(), TokenKind::KwEndState) {
            let item_start = self.cur_pos();
            match self.peek() {
                TokenKind::KwEvent => events.push(self.parse_event()),
                TokenKind::KwFunction => {
                    functions.push(self.parse_function(None, item_start));
                }
                TokenKind::Name(_) | TokenKind::KwVar => {
                    let ty = self.parse_type_ref();
                    if matches!(self.peek(), TokenKind::KwFunction) {
                        functions.push(self.parse_function(Some(ty), item_start));
                    } else {
                        let p = self.cur_pos();
                        self.error_at(p, "expected `Function` after type in state body");
                        self.sync_to_top_level();
                    }
                }
                _ => {
                    let p = self.cur_pos();
                    self.error_at(p, format!("unexpected token {:?} in state", self.peek()));
                    self.sync_to_top_level();
                }
            }
            self.skip_newlines();
        }
        self.eat(&TokenKind::KwEndState);
        let end = self.last_end();
        StateDef {
            name,
            is_auto,
            functions,
            events,
            pos: span(start, end),
        }
    }

    fn parse_param_list(&mut self) -> Vec<Parameter> {
        let mut out = Vec::new();
        if matches!(self.peek(), TokenKind::RParen) {
            return out;
        }
        loop {
            let p = self.parse_param();
            out.push(p);
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        out
    }

    fn parse_param(&mut self) -> Parameter {
        let start = self.cur_pos();
        let ty = self.parse_type_ref();
        let name = match self.peek() {
            TokenKind::Name(n) => {
                let s = n.clone();
                self.pos += 1;
                s
            }
            _ => {
                let p = self.cur_pos();
                self.error_at(p, "expected parameter name");
                "Unknown".into()
            }
        };
        let default = if self.eat(&TokenKind::Assign) {
            Some(self.parse_literal_expr_or_neg())
        } else {
            None
        };
        let end = self.last_end();
        Parameter {
            name,
            ty,
            default,
            pos: span(start, end),
        }
    }

    /// Parameter defaults are `literal` per grammar: includes negative literals.
    fn parse_literal_expr_or_neg(&mut self) -> Expr {
        if matches!(self.peek(), TokenKind::Minus) {
            // unary minus on a literal
            let start = self.cur_pos();
            self.pos += 1;
            let inner = self.parse_atom();
            let pos = span(start, self.last_end());
            return Expr::UnaryExpr {
                op: "-".into(),
                operand: Box::new(inner),
                pos,
            };
        }
        self.parse_atom()
    }

    // -----------------------------------------------------------------------
    // Statements
    // -----------------------------------------------------------------------

    fn parse_block_until(&mut self, terminators: &[TokenKind]) -> Vec<Stmt> {
        let mut out = Vec::new();
        loop {
            self.skip_newlines();
            if self.at_eof() {
                break;
            }
            if terminators
                .iter()
                .any(|t| std::mem::discriminant(self.peek()) == std::mem::discriminant(t))
            {
                break;
            }
            let before = self.pos;
            match self.parse_statement() {
                Some(s) => {
                    // Guarantee forward progress: a statement that consumed no
                    // tokens (an unhandled construct) would otherwise spin here.
                    if self.pos == before {
                        self.pos += 1;
                    }
                    out.push(s);
                }
                None => {
                    self.sync_to_top_level();
                }
            }
        }
        out
    }

    fn parse_statement(&mut self) -> Option<Stmt> {
        match self.peek() {
            TokenKind::KwIf => Some(self.parse_if()),
            TokenKind::KwWhile => Some(self.parse_while()),
            TokenKind::KwReturn => Some(self.parse_return()),
            TokenKind::Name(_) | TokenKind::KwVar => Some(self.parse_local_var_or_assign_or_expr()),
            TokenKind::KwSelf | TokenKind::KwParent | TokenKind::LParen => {
                Some(self.parse_assign_or_expr_stmt())
            }
            _ => {
                let p = self.cur_pos();
                self.error_at(
                    p,
                    format!("unexpected token {:?} in statement", self.peek()),
                );
                None
            }
        }
    }

    fn parse_if(&mut self) -> Stmt {
        let start = self.cur_pos();
        self.pos += 1; // if
        let cond = self.parse_expr();
        self.skip_newlines();
        let body =
            self.parse_block_until(&[TokenKind::KwElseIf, TokenKind::KwElse, TokenKind::KwEndIf]);
        let mut elseif_clauses = Vec::new();
        while matches!(self.peek(), TokenKind::KwElseIf) {
            let ei_start = self.cur_pos();
            self.pos += 1;
            let ei_cond = self.parse_expr();
            self.skip_newlines();
            let ei_body = self.parse_block_until(&[
                TokenKind::KwElseIf,
                TokenKind::KwElse,
                TokenKind::KwEndIf,
            ]);
            let pos = span(ei_start, self.last_end());
            elseif_clauses.push(ElseIfClause {
                condition: ei_cond,
                body: ei_body,
                pos,
            });
        }
        let else_body = if matches!(self.peek(), TokenKind::KwElse) {
            self.pos += 1;
            self.skip_newlines();
            self.parse_block_until(&[TokenKind::KwEndIf])
        } else {
            Vec::new()
        };
        self.eat(&TokenKind::KwEndIf);
        Stmt::IfStmt {
            condition: cond,
            body,
            elseif_clauses,
            else_body,
            pos: span(start, self.last_end()),
        }
    }

    fn parse_while(&mut self) -> Stmt {
        let start = self.cur_pos();
        self.pos += 1; // while
        let cond = self.parse_expr();
        self.skip_newlines();
        let body = self.parse_block_until(&[TokenKind::KwEndWhile]);
        self.eat(&TokenKind::KwEndWhile);
        Stmt::WhileStmt {
            condition: cond,
            body,
            pos: span(start, self.last_end()),
        }
    }

    fn parse_return(&mut self) -> Stmt {
        let start = self.cur_pos();
        self.pos += 1; // return
        let value = if matches!(self.peek(), TokenKind::Newline | TokenKind::Eof) {
            None
        } else {
            Some(self.parse_expr())
        };
        Stmt::ReturnStmt {
            value,
            pos: span(start, self.last_end()),
        }
    }

    /// Disambiguate local variable declaration vs. assignment vs. expression.
    /// Local var: NAME (":" NAME)? ("[]" )? NAME ...
    /// Other:     NAME (anything else)
    fn parse_local_var_or_assign_or_expr(&mut self) -> Stmt {
        if self.lookahead_is_local_var() {
            let start = self.cur_pos();
            let ty = self.parse_type_ref();
            let name = match self.peek() {
                TokenKind::Name(n) => {
                    let s = n.clone();
                    self.pos += 1;
                    s
                }
                _ => "Unknown".into(),
            };
            let value = if self.eat(&TokenKind::Assign) {
                let v = Some(self.parse_expr());
                // Optional trailing `Const` per grammar — accept and ignore.
                self.eat(&TokenKind::KwConst);
                v
            } else {
                None
            };
            return Stmt::LocalVarStmt {
                name,
                ty,
                value,
                pos: span(start, self.last_end()),
            };
        }
        self.parse_assign_or_expr_stmt()
    }

    fn lookahead_is_local_var(&self) -> bool {
        // `Var` (optionally `Var[]`) introduces a local: `Var name` / `Var[] name`.
        if matches!(self.peek(), TokenKind::KwVar) {
            let mut i = 1;
            if matches!(self.peek_at(i), TokenKind::LBracket)
                && matches!(self.peek_at(i + 1), TokenKind::RBracket)
            {
                i += 2;
            }
            return matches!(self.peek_at(i), TokenKind::Name(_));
        }
        // NAME ...
        if !matches!(self.peek(), TokenKind::Name(_)) {
            return false;
        }
        let mut i = 1usize;
        if matches!(self.peek_at(i), TokenKind::Colon)
            && matches!(self.peek_at(i + 1), TokenKind::Name(_))
        {
            i += 2;
        }
        if matches!(self.peek_at(i), TokenKind::LBracket)
            && matches!(self.peek_at(i + 1), TokenKind::RBracket)
        {
            i += 2;
        }
        matches!(self.peek_at(i), TokenKind::Name(_))
    }

    fn parse_assign_or_expr_stmt(&mut self) -> Stmt {
        let start = self.cur_pos();
        let lhs = self.parse_expr();
        if let Some(op) = match_assign_op(self.peek()) {
            self.pos += 1;
            let rhs = self.parse_expr();
            return Stmt::AssignStmt {
                target: lhs,
                op,
                value: rhs,
                pos: span(start, self.last_end()),
            };
        }
        Stmt::ExprStmt {
            expr: lhs,
            pos: span(start, self.last_end()),
        }
    }

    // -----------------------------------------------------------------------
    // Expressions (precedence climbing)
    // -----------------------------------------------------------------------

    fn parse_expr(&mut self) -> Expr {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Expr {
        let mut left = self.parse_and();
        while matches!(self.peek(), TokenKind::OrOr) {
            let start = left.pos();
            self.pos += 1;
            let right = self.parse_and();
            let end_pos = right.pos();
            left = Expr::BinaryExpr {
                left: Box::new(left),
                op: "||".into(),
                right: Box::new(right),
                pos: Pos {
                    line: start.line,
                    col: start.col,
                    end_line: end_pos.end_line,
                    end_col: end_pos.end_col,
                },
            };
        }
        left
    }

    fn parse_and(&mut self) -> Expr {
        let mut left = self.parse_cmp();
        while matches!(self.peek(), TokenKind::AndAnd) {
            let start = left.pos();
            self.pos += 1;
            let right = self.parse_cmp();
            let end_pos = right.pos();
            left = Expr::BinaryExpr {
                left: Box::new(left),
                op: "&&".into(),
                right: Box::new(right),
                pos: Pos {
                    line: start.line,
                    col: start.col,
                    end_line: end_pos.end_line,
                    end_col: end_pos.end_col,
                },
            };
        }
        left
    }

    fn parse_cmp(&mut self) -> Expr {
        let mut left = self.parse_add();
        while let Some(op) = match_cmp_op(self.peek()) {
            self.pos += 1;
            let start = left.pos();
            let right = self.parse_add();
            let end_pos = right.pos();
            left = Expr::BinaryExpr {
                left: Box::new(left),
                op,
                right: Box::new(right),
                pos: Pos {
                    line: start.line,
                    col: start.col,
                    end_line: end_pos.end_line,
                    end_col: end_pos.end_col,
                },
            };
        }
        left
    }

    fn parse_add(&mut self) -> Expr {
        let mut left = self.parse_mul();
        loop {
            let op = match self.peek() {
                TokenKind::Plus => "+",
                TokenKind::Minus => "-",
                _ => break,
            };
            self.pos += 1;
            let start = left.pos();
            let right = self.parse_mul();
            let end_pos = right.pos();
            left = Expr::BinaryExpr {
                left: Box::new(left),
                op: op.into(),
                right: Box::new(right),
                pos: Pos {
                    line: start.line,
                    col: start.col,
                    end_line: end_pos.end_line,
                    end_col: end_pos.end_col,
                },
            };
        }
        left
    }

    fn parse_mul(&mut self) -> Expr {
        let mut left = self.parse_unary();
        loop {
            let op = match self.peek() {
                TokenKind::Star => "*",
                TokenKind::Slash => "/",
                TokenKind::Percent => "%",
                _ => break,
            };
            self.pos += 1;
            let start = left.pos();
            let right = self.parse_unary();
            let end_pos = right.pos();
            left = Expr::BinaryExpr {
                left: Box::new(left),
                op: op.into(),
                right: Box::new(right),
                pos: Pos {
                    line: start.line,
                    col: start.col,
                    end_line: end_pos.end_line,
                    end_col: end_pos.end_col,
                },
            };
        }
        left
    }

    fn parse_unary(&mut self) -> Expr {
        match self.peek() {
            TokenKind::Bang => {
                let start = self.cur_pos();
                self.pos += 1;
                let inner = self.parse_unary();
                Expr::UnaryExpr {
                    op: "!".into(),
                    operand: Box::new(inner),
                    pos: span(start, self.last_end()),
                }
            }
            TokenKind::Minus => {
                let start = self.cur_pos();
                self.pos += 1;
                let inner = self.parse_unary();
                Expr::UnaryExpr {
                    op: "-".into(),
                    operand: Box::new(inner),
                    pos: span(start, self.last_end()),
                }
            }
            _ => self.parse_cast(),
        }
    }

    fn parse_cast(&mut self) -> Expr {
        let start = self.cur_pos();
        let mut left = self.parse_postfix();
        loop {
            match self.peek() {
                TokenKind::KwAs => {
                    self.pos += 1;
                    let target = self.parse_type_ref();
                    left = Expr::CastExpr {
                        expr: Box::new(left),
                        target_type: target,
                        pos: span(start, self.last_end()),
                    };
                }
                TokenKind::KwIs => {
                    // Consumed but discarded (matches Python transformer behavior).
                    self.pos += 1;
                    let _ = self.parse_type_ref();
                }
                _ => break,
            }
        }
        left
    }

    fn parse_postfix(&mut self) -> Expr {
        let start = self.cur_pos();
        let mut left = self.parse_atom();
        loop {
            match self.peek() {
                TokenKind::Dot => {
                    self.pos += 1;
                    let member = match self.peek() {
                        TokenKind::Name(n) => {
                            let s = n.clone();
                            self.pos += 1;
                            s
                        }
                        TokenKind::KwLength => {
                            self.pos += 1;
                            "Length".into()
                        }
                        _ => {
                            let p = self.cur_pos();
                            self.error_at(p, "expected member name after `.`");
                            "Unknown".into()
                        }
                    };
                    if matches!(self.peek(), TokenKind::LParen) {
                        self.pos += 1;
                        let (args, _names) = self.parse_arg_list();
                        self.expect(&TokenKind::RParen, "`)`");
                        left = Expr::DotCallExpr {
                            object: Box::new(left),
                            method: member,
                            args,
                            pos: span(start, self.last_end()),
                        };
                    } else {
                        left = Expr::DotExpr {
                            object: Box::new(left),
                            member,
                            pos: span(start, self.last_end()),
                        };
                    }
                }
                TokenKind::LBracket => {
                    self.pos += 1;
                    let index = self.parse_expr();
                    self.expect(&TokenKind::RBracket, "`]`");
                    left = Expr::ArrayAccessExpr {
                        array: Box::new(left),
                        index: Box::new(index),
                        pos: span(start, self.last_end()),
                    };
                }
                TokenKind::LParen => {
                    self.pos += 1;
                    let (args, arg_names) = self.parse_arg_list();
                    self.expect(&TokenKind::RParen, "`)`");
                    left = match left {
                        Expr::NameExpr { name, .. } => Expr::CallExpr {
                            function: name,
                            args,
                            arg_names,
                            pos: span(start, self.last_end()),
                        },
                        Expr::DotExpr { object, member, .. } => Expr::DotCallExpr {
                            object,
                            method: member,
                            args,
                            pos: span(start, self.last_end()),
                        },
                        other => Expr::CallExpr {
                            function: "<expr>".into(),
                            args,
                            arg_names,
                            pos: other.pos(),
                        },
                    };
                }
                _ => break,
            }
        }
        left
    }

    /// Parse a call argument list, returning the value expressions and, parallel
    /// to them, each argument's explicit `name =` (or `None` if positional).
    fn parse_arg_list(&mut self) -> (Vec<Expr>, Vec<Option<String>>) {
        let mut out = Vec::new();
        let mut names = Vec::new();
        if matches!(self.peek(), TokenKind::RParen) {
            return (out, names);
        }
        loop {
            // Named arg: NAME = expr.
            if let TokenKind::Name(n) = self.peek().clone() {
                if matches!(self.peek_at(1), TokenKind::Assign) {
                    self.pos += 2;
                    out.push(self.parse_expr());
                    names.push(Some(n));
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                    continue;
                }
            }
            out.push(self.parse_expr());
            names.push(None);
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        (out, names)
    }

    fn parse_atom(&mut self) -> Expr {
        let start = self.cur_pos();
        match self.peek().clone() {
            TokenKind::LParen => {
                self.pos += 1;
                let inner = self.parse_expr();
                self.expect(&TokenKind::RParen, "`)`");
                inner
            }
            TokenKind::KwNew => {
                self.pos += 1;
                let ty = self.parse_type_name();
                if matches!(self.peek(), TokenKind::LBracket) {
                    self.pos += 1;
                    let size = self.parse_expr();
                    self.expect(&TokenKind::RBracket, "`]`");
                    Expr::NewArrayExpr {
                        element_type: ty,
                        size: Box::new(size),
                        pos: span(start, self.last_end()),
                    }
                } else {
                    // new struct: represent as call `new <Type>` (matches Python).
                    Expr::CallExpr {
                        function: format!("new {ty}"),
                        args: Vec::new(),
                        arg_names: Vec::new(),
                        pos: span(start, self.last_end()),
                    }
                }
            }
            TokenKind::KwSelf => {
                self.pos += 1;
                Expr::NameExpr {
                    name: "Self".into(),
                    pos: span(start, self.last_end()),
                }
            }
            TokenKind::KwParent => {
                self.pos += 1;
                Expr::ParentExpr {
                    pos: span(start, self.last_end()),
                }
            }
            TokenKind::KwNone => {
                self.pos += 1;
                Expr::LiteralExpr {
                    value: LiteralValue::Null,
                    ty: "none".into(),
                    pos: span(start, self.last_end()),
                }
            }
            TokenKind::KwTrue => {
                self.pos += 1;
                Expr::LiteralExpr {
                    value: LiteralValue::Bool(true),
                    ty: "bool".into(),
                    pos: span(start, self.last_end()),
                }
            }
            TokenKind::KwFalse => {
                self.pos += 1;
                Expr::LiteralExpr {
                    value: LiteralValue::Bool(false),
                    ty: "bool".into(),
                    pos: span(start, self.last_end()),
                }
            }
            TokenKind::Int(v) => {
                self.pos += 1;
                Expr::LiteralExpr {
                    value: LiteralValue::Int(v),
                    ty: "int".into(),
                    pos: span(start, self.last_end()),
                }
            }
            TokenKind::Float(v) => {
                self.pos += 1;
                Expr::LiteralExpr {
                    value: LiteralValue::Float(v),
                    ty: "float".into(),
                    pos: span(start, self.last_end()),
                }
            }
            TokenKind::Str(s) => {
                self.pos += 1;
                Expr::LiteralExpr {
                    value: LiteralValue::Str(s),
                    ty: "string".into(),
                    pos: span(start, self.last_end()),
                }
            }
            TokenKind::Name(n) => {
                self.pos += 1;
                // A `:`-namespaced script name used as a value / static-call
                // receiver (e.g. `AutoTestShared:Utilities.SetGameHour(0)`).
                let mut name = n;
                while matches!(self.peek(), TokenKind::Colon)
                    && matches!(self.peek_at(1), TokenKind::Name(_))
                {
                    self.pos += 1; // ':'
                    if let TokenKind::Name(seg) = self.peek() {
                        name.push(':');
                        name.push_str(seg);
                        self.pos += 1;
                    }
                }
                Expr::NameExpr {
                    name,
                    pos: span(start, self.last_end()),
                }
            }
            other => {
                self.error_at(start, format!("unexpected token {:?} in expression", other));
                Expr::LiteralExpr {
                    value: LiteralValue::Null,
                    ty: "none".into(),
                    pos: span(start, start),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct ScriptHeader {
    name: String,
    parent: Option<String>,
    flags: Vec<String>,
    pos: Pos,
}

enum TopLevel {
    Import(ImportNode),
    Property(Vec<PropertyDef>),
    Variable(VariableDef),
    Function(FunctionDef),
    Event(EventDef),
    State(StateDef),
    Struct(StructDef),
    Discard,
}

fn match_script_flag(t: &TokenKind) -> Option<String> {
    Some(
        match t {
            TokenKind::KwNative => "Native",
            TokenKind::KwHidden => "Hidden",
            TokenKind::KwConditional => "Conditional",
            TokenKind::KwConst => "Const",
            TokenKind::KwDefault => "Default",
            TokenKind::KwBetaOnly => "BetaOnly",
            TokenKind::KwDebugOnly => "DebugOnly",
            _ => return None,
        }
        .into(),
    )
}

fn match_prop_flag(t: &TokenKind) -> Option<String> {
    Some(
        match t {
            TokenKind::KwAuto => "Auto",
            TokenKind::KwAutoReadOnly => "AutoReadOnly",
            TokenKind::KwConst => "Const",
            TokenKind::KwMandatory => "Mandatory",
            TokenKind::KwHidden => "Hidden",
            TokenKind::KwConditional => "Conditional",
            _ => return None,
        }
        .into(),
    )
}

fn match_var_flag(t: &TokenKind) -> Option<String> {
    Some(
        match t {
            TokenKind::KwConst => "Const",
            TokenKind::KwConditional => "Conditional",
            _ => return None,
        }
        .into(),
    )
}

/// Struct members accept the full set of member flags (Hidden/Mandatory in
/// addition to Const/Conditional), unlike script variables.
fn match_struct_member_flag(t: &TokenKind) -> Option<String> {
    Some(
        match t {
            TokenKind::KwConst => "Const",
            TokenKind::KwConditional => "Conditional",
            TokenKind::KwHidden => "Hidden",
            TokenKind::KwMandatory => "Mandatory",
            _ => return None,
        }
        .into(),
    )
}

fn match_cmp_op(t: &TokenKind) -> Option<String> {
    Some(
        match t {
            TokenKind::Eq => "==",
            TokenKind::Neq => "!=",
            TokenKind::Lt => "<",
            TokenKind::Gt => ">",
            TokenKind::Lte => "<=",
            TokenKind::Gte => ">=",
            _ => return None,
        }
        .into(),
    )
}

fn match_assign_op(t: &TokenKind) -> Option<String> {
    Some(
        match t {
            TokenKind::Assign => "=",
            TokenKind::PlusAssign => "+=",
            TokenKind::MinusAssign => "-=",
            TokenKind::MulAssign => "*=",
            TokenKind::DivAssign => "/=",
            TokenKind::ModAssign => "%=",
            _ => return None,
        }
        .into(),
    )
}

// ---------------------------------------------------------------------------
// Tests — mirror plugins/papyrus_lsp_plugin/.../tests/test_parser.py
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(text: &str) -> ScriptNode {
        let r = parse_script(text);
        if !r.errors.is_empty() {
            panic!("unexpected errors: {:?}", r.errors);
        }
        r.ast.expect("expected AST")
    }

    #[test]
    fn empty_string_produces_error() {
        let r = parse_script("");
        assert!(r.ast.is_none());
        assert!(!r.errors.is_empty());
    }

    #[test]
    fn parse_simple_script_header() {
        let s = parse_ok("ScriptName Foo extends Bar\n");
        assert_eq!(s.name, "Foo");
        assert_eq!(s.parent.as_deref(), Some("Bar"));
    }

    #[test]
    fn parse_native_hidden_flags() {
        let s = parse_ok("Scriptname Game Native Hidden\n");
        assert_eq!(s.name, "Game");
        assert!(s.flags.iter().any(|f| f == "Native"));
        assert!(s.flags.iter().any(|f| f == "Hidden"));
    }

    #[test]
    fn parse_namespaced_script_name() {
        let s = parse_ok("ScriptName B21:B21_AC_AmmoConverter extends ObjectReference\n");
        assert_eq!(s.name, "B21:B21_AC_AmmoConverter");
        assert_eq!(s.parent.as_deref(), Some("ObjectReference"));
    }

    #[test]
    fn parse_struct_with_member_flags_and_defaults() {
        let s = parse_ok(
            "ScriptName Foo\nStruct Data\n  Int A\n  Bool B hidden\n  Float C = 0.5\n  Int D = -1 Const\nEndStruct\n",
        );
        let st = s.structs.iter().find(|s| s.name == "Data").expect("struct");
        assert_eq!(st.members.len(), 4);
        assert_eq!(st.members[1].name, "B");
        assert!(st.members[1].flags.iter().any(|f| f == "Hidden"));
        assert!(st.members[3].flags.iter().any(|f| f == "Const"));
    }

    #[test]
    fn parse_namespaced_static_call_receiver() {
        // A `:`-namespaced script name as a static-call receiver in a statement.
        let s = parse_ok(
            "ScriptName Foo\nFunction F()\n  AutoTestShared:Utilities.SetGameHour(0)\nEndFunction\n",
        );
        let f = s.functions.iter().find(|f| f.name == "F").expect("fn");
        assert_eq!(f.body.len(), 1);
    }

    #[test]
    fn parse_multi_segment_namespaced_type() {
        // FO76 types can carry several `:` namespace segments.
        let s = parse_ok("ScriptName Foo\nquests:_default:progressbar:masterscript ProgressBar\n");
        let v = s
            .variables
            .iter()
            .find(|v| v.name == "ProgressBar")
            .expect("var");
        assert_eq!(v.ty, "quests:_default:progressbar:masterscript");
    }

    #[test]
    fn case_insensitive_keywords() {
        let s = parse_ok("scriptname Foo EXTENDS Bar\n");
        assert_eq!(s.name, "Foo");
        assert_eq!(s.parent.as_deref(), Some("Bar"));
    }

    #[test]
    fn auto_property() {
        let s = parse_ok("ScriptName Foo\nInt Property MyProp Auto\n");
        assert_eq!(s.properties.len(), 1);
        assert_eq!(s.properties[0].name, "MyProp");
        assert_eq!(s.properties[0].ty, "Int");
        assert!(s.properties[0].flags.iter().any(|f| f == "Auto"));
    }

    #[test]
    fn auto_const_mandatory() {
        let s = parse_ok("ScriptName Foo\nKeyword Property pLink Auto Const Mandatory\n");
        let p = &s.properties[0];
        assert_eq!(p.ty, "Keyword");
        for want in ["Auto", "Const", "Mandatory"] {
            assert!(p.flags.iter().any(|f| f == want), "missing {want}");
        }
    }

    #[test]
    fn property_default_value() {
        let s = parse_ok("ScriptName Foo\nFloat Property MyVal = 1.5 Auto\n");
        let p = &s.properties[0];
        assert_eq!(p.name, "MyVal");
        assert!(p.default.is_some());
    }

    #[test]
    fn array_property() {
        let s = parse_ok("ScriptName Foo\nString[] Property Names Auto\n");
        assert_eq!(s.properties[0].ty, "String[]");
    }

    #[test]
    fn simple_function() {
        let s = parse_ok("ScriptName Foo\nFunction DoThing()\nEndFunction\n");
        assert_eq!(s.functions.len(), 1);
        assert_eq!(s.functions[0].name, "DoThing");
    }

    #[test]
    fn function_with_return_type() {
        let s = parse_ok("ScriptName Foo\nInt Function GetCount()\n  Return 5\nEndFunction\n");
        assert_eq!(s.functions[0].return_type, "Int");
    }

    #[test]
    fn native_global_function_no_body() {
        let s = parse_ok("ScriptName Foo Native Hidden\nFunction DoThing() native global\n");
        let f = &s.functions[0];
        assert!(f.is_native);
        assert!(f.is_global);
        assert!(f.body.is_empty());
    }

    #[test]
    fn function_with_params() {
        let s =
            parse_ok("ScriptName Foo\nFunction DoThing(Int aiCount, String asName)\nEndFunction\n");
        let f = &s.functions[0];
        assert_eq!(f.params.len(), 2);
        assert_eq!(f.params[0].name, "aiCount");
        assert_eq!(f.params[0].ty, "Int");
    }

    #[test]
    fn function_default_params() {
        let s = parse_ok(
            "ScriptName Foo\nFunction DoThing(Float afPower = 0.5, Bool abFlag = true)\nEndFunction\n",
        );
        let f = &s.functions[0];
        assert!(f.params[0].default.is_some());
        assert!(f.params[1].default.is_some());
    }

    #[test]
    fn event_definition() {
        let s =
            parse_ok("ScriptName Foo\nEvent OnActivate(ObjectReference akActionRef)\nEndEvent\n");
        assert_eq!(s.events.len(), 1);
        assert_eq!(s.events[0].name, "OnActivate");
    }

    #[test]
    fn dot_call_expression() {
        let r = parse_script("ScriptName Foo\nFunction Bar()\n  Game.GetPlayer()\nEndFunction\n");
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
    }

    #[test]
    fn chained_dot_call() {
        let r = parse_script(
            "ScriptName Foo\nFunction Bar()\n  Game.GetPlayer().GetActorBase()\nEndFunction\n",
        );
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
    }

    #[test]
    fn cast_expression() {
        let r =
            parse_script("ScriptName Foo\nFunction Bar()\n  Int x = akRef as Int\nEndFunction\n");
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
    }

    #[test]
    fn array_access_expression() {
        let r = parse_script(
            "ScriptName Foo\nFunction Bar()\n  String[] arr = new String[5]\n  String s = arr[0]\nEndFunction\n",
        );
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
    }

    #[test]
    fn comparison_and_logical() {
        let r = parse_script(
            "ScriptName Foo\nFunction Bar()\n  If x > 0 && y != None\n  EndIf\nEndFunction\n",
        );
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
    }

    #[test]
    fn string_concat() {
        let r = parse_script(
            "ScriptName Foo\nFunction Bar()\n  String s = \"hello \" + \"world\"\nEndFunction\n",
        );
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
    }

    #[test]
    fn auto_state_block() {
        let s = parse_ok(
            "ScriptName Foo\nAuto State Waiting\n  Event OnActivate(ObjectReference akRef)\n  EndEvent\nEndState\n",
        );
        assert_eq!(s.states.len(), 1);
        assert!(s.states[0].is_auto);
        assert_eq!(s.states[0].name, "Waiting");
    }

    #[test]
    fn parse_from_text_basic_property() {
        let s = parse_ok("ScriptName Foo\n\nInt Property Bar Auto\n");
        assert_eq!(s.name, "Foo");
        assert_eq!(s.properties.len(), 1);
    }

    #[test]
    fn full_property_with_getter_setter() {
        let s = parse_ok(
            "ScriptName Foo\n\
             Int Property Counter\n\
             Int Function Get()\n\
               Return 0\n\
             EndFunction\n\
             Function Set(Int aiVal)\n\
             EndFunction\n\
             EndProperty\n",
        );
        assert_eq!(s.properties.len(), 1);
        assert!(s.properties[0].getter.is_some());
        assert!(s.properties[0].setter.is_some());
    }

    #[test]
    fn import_statement() {
        let s = parse_ok("ScriptName Foo\nImport Game\n");
        assert_eq!(s.imports.len(), 1);
        assert_eq!(s.imports[0].script_name, "Game");
    }

    #[test]
    fn variable_with_init() {
        let s = parse_ok("ScriptName Foo\nInt myVar = 42\n");
        assert_eq!(s.variables.len(), 1);
        assert_eq!(s.variables[0].name, "myVar");
        assert_eq!(s.variables[0].ty, "Int");
        assert!(s.variables[0].value.is_some());
    }

    #[test]
    fn var_array_local_does_not_hang() {
        // Regression: `Var[]` local declaration used to infinite-loop the
        // statement parser (lookahead only matched `Var name`, not `Var[] name`).
        let r = parse_script(
            "ScriptName Foo\nFunction Bar()\n  Var[] kargs = new Var[3]\nEndFunction\n",
        );
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
        let body = &r.ast.unwrap().functions[0].body;
        assert!(matches!(body[0], Stmt::LocalVarStmt { .. }));
    }

    #[test]
    fn while_loop() {
        let r = parse_script(
            "ScriptName Foo\nFunction Bar()\n  While x < 10\n    x = x + 1\n  EndWhile\nEndFunction\n",
        );
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
    }

    #[test]
    fn if_elseif_else() {
        let r = parse_script(
            "ScriptName Foo\nFunction Bar()\n  If x == 0\n    x = 1\n  ElseIf x == 1\n    x = 2\n  Else\n    x = 3\n  EndIf\nEndFunction\n",
        );
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
    }

    #[test]
    fn assignment_with_compound_op() {
        let r = parse_script("ScriptName Foo\nFunction Bar()\n  x += 1\nEndFunction\n");
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
    }

    #[test]
    fn remote_event_with_dotted_name() {
        let s = parse_ok("ScriptName Foo\nEvent Actor.OnSit(Actor akSender)\nEndEvent\n");
        assert_eq!(s.events[0].name, "Actor.OnSit");
    }

    #[test]
    fn group_extracts_properties() {
        let s = parse_ok(
            "ScriptName Foo\nGroup MyGroup\n  Int Property A Auto\n  Int Property B Auto\nEndGroup\n",
        );
        assert_eq!(s.properties.len(), 2);
    }

    #[test]
    fn struct_def_is_discarded() {
        let r = parse_script("ScriptName Foo\nStruct Point\n  Float X\n  Float Y\nEndStruct\n");
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
    }

    #[test]
    fn unary_minus_in_expression() {
        let r = parse_script("ScriptName Foo\nFunction Bar()\n  Int x = -5\nEndFunction\n");
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
    }

    #[test]
    fn string_literal_with_default_param_negative() {
        let s = parse_ok("ScriptName Foo\nFunction Bar(Int x = -1, Float y = 2.5)\nEndFunction\n");
        assert!(s.functions[0].params[0].default.is_some());
    }

    #[test]
    fn comment_handling_does_not_break_parsing() {
        let r =
            parse_script("; top-level comment\nScriptName Foo extends Bar ; trailing\n; another\n");
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
        assert_eq!(r.ast.unwrap().name, "Foo");
    }
}

/// Validate that the .psc filename matches the `Scriptname` declaration.
/// Mirrors `validate_filename` in `py_creation_lib/python/creation_lib/papyrus_lsp/parser.py:737-792`.
pub fn validate_filename(path: &str, script_name: &str) -> Option<ParseError> {
    if path.is_empty() || script_name.is_empty() {
        return None;
    }
    let path_buf = std::path::Path::new(path);
    let stem = path_buf.file_stem().and_then(|s| s.to_str()).unwrap_or("");

    if script_name.contains(':') {
        let parts: Vec<&str> = script_name.split(':').collect();
        let expected_stem = *parts.last().unwrap();
        let expected_folders = &parts[..parts.len() - 1];

        if !stem.eq_ignore_ascii_case(expected_stem) {
            return Some(ParseError {
                line: 1,
                col: 0,
                message: format!(
                    "Filename '{stem}.psc' does not match Scriptname '{script_name}' (expected '{expected_stem}.psc')"
                ),
            });
        }

        let mut current = path_buf.parent();
        for expected in expected_folders.iter().rev() {
            let actual = current
                .and_then(|c| c.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if !actual.eq_ignore_ascii_case(expected) {
                let expected_path = format!("{}/{}.psc", expected_folders.join("/"), expected_stem);
                return Some(ParseError {
                    line: 1,
                    col: 0,
                    message: format!(
                        "Script '{script_name}' should be at '.../{expected_path}' but folder '{actual}' does not match '{expected}'"
                    ),
                });
            }
            current = current.and_then(|c| c.parent());
        }
        None
    } else if !stem.eq_ignore_ascii_case(script_name) {
        Some(ParseError {
            line: 1,
            col: 0,
            message: format!(
                "Filename '{stem}.psc' does not match Scriptname '{script_name}' (expected '{script_name}.psc')"
            ),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod filename_tests {
    use super::*;

    #[test]
    fn matching_simple_name() {
        assert!(validate_filename("/path/to/MyScript.psc", "MyScript").is_none());
    }

    #[test]
    fn matching_case_insensitive() {
        assert!(validate_filename("/path/to/myscript.psc", "MyScript").is_none());
    }

    #[test]
    fn mismatched_simple_name() {
        let err = validate_filename("/path/to/WrongName.psc", "MyScript").unwrap();
        assert!(err.message.contains("WrongName"));
        assert!(err.message.contains("MyScript"));
    }

    #[test]
    fn matching_namespace() {
        assert!(validate_filename("/path/to/B21/TestScript.psc", "B21:TestScript").is_none());
    }

    #[test]
    fn mismatched_namespace_folder() {
        let err = validate_filename("/path/to/Wrong/TestScript.psc", "B21:TestScript").unwrap();
        assert!(err.message.contains("B21"));
    }

    #[test]
    fn matching_fragment_namespace() {
        assert!(
            validate_filename(
                "/Scripts/Source/Base/Fragments/Quests/QF_MQ101_0001ED86.psc",
                "Fragments:Quests:QF_MQ101_0001ED86",
            )
            .is_none()
        );
    }

    #[test]
    fn empty_path_returns_none() {
        assert!(validate_filename("", "Foo").is_none());
    }

    #[test]
    fn empty_name_returns_none() {
        assert!(validate_filename("/path/to/Foo.psc", "").is_none());
    }
}
