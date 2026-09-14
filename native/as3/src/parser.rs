//! Recursive-descent parser producing the [`crate::ast`] tree.
//!
//! Written from the AS3 grammar in Adobe's language reference, not from another
//! compiler's grammar.
//!
//! The parser accepts more than codegen can lower and records those constructs
//! as [`crate::ast::Stmt::Unsupported`], so "not valid AS3" and "not supported
//! by this compiler" stay separate diagnostics.

use crate::ast::*;
use crate::diag::{Diagnostic, Result, Span};
use crate::lexer::{Keyword, Punct, Token, TokenKind, tokenize};

pub fn parse(src: &str) -> Result<CompilationUnit> {
    let tokens = tokenize(src)?;
    Parser::new(tokens).compilation_unit()
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, at: 0 }
    }

    fn peek(&self) -> &TokenKind {
        &self.tokens[self.at].kind
    }

    fn peek_at(&self, ahead: usize) -> &TokenKind {
        let i = (self.at + ahead).min(self.tokens.len() - 1);
        &self.tokens[i].kind
    }

    fn span(&self) -> Span {
        self.tokens[self.at].span
    }

    fn prev_span(&self) -> Span {
        self.tokens[self.at.saturating_sub(1)].span
    }

    fn bump(&mut self) -> TokenKind {
        let kind = self.tokens[self.at].kind.clone();
        if self.at + 1 < self.tokens.len() {
            self.at += 1;
        }
        kind
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), TokenKind::Eof)
    }

    fn check_punct(&self, p: Punct) -> bool {
        matches!(self.peek(), TokenKind::Punct(q) if *q == p)
    }

    fn check_kw(&self, k: Keyword) -> bool {
        matches!(self.peek(), TokenKind::Keyword(q) if *q == k)
    }

    /// True when the current token is a contextual keyword spelled `name` —
    /// `static`, `dynamic`, `get`, and friends, which lex as identifiers.
    fn check_ident(&self, name: &str) -> bool {
        matches!(self.peek(), TokenKind::Ident(s) if s == name)
    }

    fn eat_punct(&mut self, p: Punct) -> bool {
        if self.check_punct(p) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, k: Keyword) -> bool {
        if self.check_kw(k) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect_punct(&mut self, p: Punct) -> Result<Span> {
        if self.check_punct(p) {
            let span = self.span();
            self.bump();
            Ok(span)
        } else {
            Err(Diagnostic::parse(
                format!(
                    "expected `{}`, found {}",
                    p.as_str(),
                    self.peek().describe()
                ),
                self.span(),
            ))
        }
    }

    fn expect_kw(&mut self, k: Keyword) -> Result<Span> {
        if self.check_kw(k) {
            let span = self.span();
            self.bump();
            Ok(span)
        } else {
            Err(Diagnostic::parse(
                format!(
                    "expected `{}`, found {}",
                    k.as_str(),
                    self.peek().describe()
                ),
                self.span(),
            ))
        }
    }

    /// Consume an identifier. Contextual keywords lex as identifiers already,
    /// so no special-casing is needed here.
    fn expect_ident(&mut self) -> Result<String> {
        match self.peek().clone() {
            TokenKind::Ident(name) => {
                self.bump();
                Ok(name)
            }
            other => Err(Diagnostic::parse(
                format!("expected an identifier, found {}", other.describe()),
                self.span(),
            )),
        }
    }

    // ---------------------------------------------------------------- top level

    fn compilation_unit(&mut self) -> Result<CompilationUnit> {
        let mut packages = Vec::new();
        while !self.at_eof() {
            if self.check_kw(Keyword::Package) {
                packages.push(self.package()?);
            } else {
                return Err(Diagnostic::parse(
                    format!(
                        "expected `package` at the top level, found {}",
                        self.peek().describe()
                    ),
                    self.span(),
                ));
            }
        }
        if packages.is_empty() {
            return Err(Diagnostic::parse(
                "source file declares no package",
                self.span(),
            ));
        }
        Ok(CompilationUnit { packages })
    }

    fn package(&mut self) -> Result<Package> {
        let start = self.expect_kw(Keyword::Package)?;
        let name = if self.check_punct(Punct::LBrace) {
            String::new()
        } else {
            self.dotted_name()?.joined()
        };
        self.expect_punct(Punct::LBrace)?;

        let mut imports = Vec::new();
        let mut classes = Vec::new();
        while !self.check_punct(Punct::RBrace) {
            if self.at_eof() {
                return Err(Diagnostic::parse("unterminated package block", self.span()));
            }
            if self.eat_punct(Punct::Semi) {
                continue;
            }
            if self.check_kw(Keyword::Import) {
                imports.push(self.import()?);
                continue;
            }
            if self.check_kw(Keyword::Use) {
                // `use namespace X;` — recognised so it does not derail the
                // parse, but it has no effect on resolution in this phase.
                while !self.check_punct(Punct::Semi) && !self.at_eof() {
                    self.bump();
                }
                self.eat_punct(Punct::Semi);
                continue;
            }
            let modifiers = self.modifiers()?;
            if self.check_kw(Keyword::Class) || self.check_kw(Keyword::Interface) {
                classes.push(self.class(modifiers)?);
                continue;
            }
            return Err(Diagnostic::parse(
                format!(
                    "expected a class or interface declaration, found {}",
                    self.peek().describe()
                ),
                self.span(),
            ));
        }
        let end = self.expect_punct(Punct::RBrace)?;
        Ok(Package {
            name,
            imports,
            classes,
            span: Span::new(start.start, end.end),
        })
    }

    fn import(&mut self) -> Result<ImportDecl> {
        let start = self.expect_kw(Keyword::Import)?;
        let mut parts = vec![self.expect_ident()?];
        let mut wildcard = false;
        while self.eat_punct(Punct::Dot) {
            if self.eat_punct(Punct::Star) {
                wildcard = true;
                break;
            }
            parts.push(self.expect_ident()?);
        }
        let end = self.prev_span();
        self.eat_punct(Punct::Semi);
        let span = Span::new(start.start, end.end);
        Ok(ImportDecl {
            name: DottedName { parts, span },
            wildcard,
            span,
        })
    }

    fn dotted_name(&mut self) -> Result<DottedName> {
        let start = self.span();
        let mut parts = vec![self.expect_ident()?];
        while self.check_punct(Punct::Dot) {
            // Do not consume the dot of a `Vector.<T>` — that is a type
            // application, not a further name component.
            if matches!(self.peek_at(1), TokenKind::Punct(Punct::Lt)) {
                break;
            }
            self.bump();
            parts.push(self.expect_ident()?);
        }
        Ok(DottedName {
            parts,
            span: Span::new(start.start, self.prev_span().end),
        })
    }

    fn modifiers(&mut self) -> Result<Modifiers> {
        let start = self.span();
        let mut m = Modifiers::default();
        loop {
            let visibility = if self.check_kw(Keyword::Public) {
                Some(Visibility::Public)
            } else if self.check_kw(Keyword::Private) {
                Some(Visibility::Private)
            } else if self.check_kw(Keyword::Protected) {
                Some(Visibility::Protected)
            } else if self.check_kw(Keyword::Internal) {
                Some(Visibility::Internal)
            } else {
                None
            };
            if let Some(v) = visibility {
                self.bump();
                m.visibility = Some(v);
                continue;
            }
            if self.eat_kw(Keyword::Native) {
                m.is_native = true;
                continue;
            }
            // Contextual keywords: only a modifier when a declaration keyword
            // follows, otherwise `static` is a perfectly good variable name.
            let contextual = match self.peek() {
                TokenKind::Ident(s) => match s.as_str() {
                    "static" | "final" | "dynamic" | "override" => Some(s.clone()),
                    _ => None,
                },
                _ => None,
            };
            let Some(word) = contextual else { break };
            if !self.starts_declaration(1) {
                break;
            }
            self.bump();
            match word.as_str() {
                "static" => m.is_static = true,
                "final" => m.is_final = true,
                "dynamic" => m.is_dynamic = true,
                "override" => m.is_override = true,
                _ => unreachable!("word came from the match above"),
            }
        }
        m.span = Span::new(start.start, self.prev_span().end);
        Ok(m)
    }

    /// Whether the token `ahead` positions from the cursor can begin a
    /// declaration — used to tell the modifier `static` from the identifier
    /// `static`.
    fn starts_declaration(&self, ahead: usize) -> bool {
        match self.peek_at(ahead) {
            TokenKind::Keyword(
                Keyword::Class
                | Keyword::Interface
                | Keyword::Function
                | Keyword::Var
                | Keyword::Const
                | Keyword::Public
                | Keyword::Private
                | Keyword::Protected
                | Keyword::Internal
                | Keyword::Native,
            ) => true,
            TokenKind::Ident(s) => {
                matches!(s.as_str(), "static" | "final" | "dynamic" | "override")
            }
            _ => false,
        }
    }

    fn class(&mut self, modifiers: Modifiers) -> Result<ClassDecl> {
        let start = self.span();
        let is_interface = self.check_kw(Keyword::Interface);
        if is_interface {
            self.bump();
        } else {
            self.expect_kw(Keyword::Class)?;
        }
        let name = self.expect_ident()?;

        let mut extends = None;
        let mut implements = Vec::new();
        if self.eat_kw(Keyword::Extends) {
            if is_interface {
                // `interface A extends B, C` — every name is a conformance.
                implements.push(self.dotted_name()?);
                while self.eat_punct(Punct::Comma) {
                    implements.push(self.dotted_name()?);
                }
            } else {
                extends = Some(self.dotted_name()?);
            }
        }
        if self.eat_kw(Keyword::Implements) {
            implements.push(self.dotted_name()?);
            while self.eat_punct(Punct::Comma) {
                implements.push(self.dotted_name()?);
            }
        }

        self.expect_punct(Punct::LBrace)?;
        let mut members = Vec::new();
        while !self.check_punct(Punct::RBrace) {
            if self.at_eof() {
                return Err(Diagnostic::parse("unterminated class body", self.span()));
            }
            if self.eat_punct(Punct::Semi) {
                continue;
            }
            members.push(self.member()?);
        }
        let end = self.expect_punct(Punct::RBrace)?;

        Ok(ClassDecl {
            modifiers,
            is_interface,
            name,
            extends,
            implements,
            members,
            span: Span::new(start.start, end.end),
        })
    }

    fn member(&mut self) -> Result<Member> {
        let modifiers = self.modifiers()?;
        if self.check_kw(Keyword::Function) {
            return Ok(Member::Function(self.function(modifiers)?));
        }
        if self.check_kw(Keyword::Var) || self.check_kw(Keyword::Const) {
            let decl = self.var_decl(modifiers)?;
            self.eat_punct(Punct::Semi);
            return Ok(Member::Var(*decl));
        }
        Err(Diagnostic::parse(
            format!(
                "expected `function`, `var` or `const` in a class body, found {}",
                self.peek().describe()
            ),
            self.span(),
        ))
    }

    fn function(&mut self, modifiers: Modifiers) -> Result<FunctionDecl> {
        let start = self.expect_kw(Keyword::Function)?;

        // `function get x():int` — `get`/`set` are only accessors when another
        // identifier follows; `function get():void` is a method called `get`.
        let mut accessor = Accessor::None;
        if (self.check_ident("get") || self.check_ident("set"))
            && matches!(self.peek_at(1), TokenKind::Ident(_))
        {
            accessor = if self.check_ident("get") {
                Accessor::Getter
            } else {
                Accessor::Setter
            };
            self.bump();
        }

        let name = self.expect_ident()?;
        let sig = self.function_signature()?;
        let body = if self.check_punct(Punct::LBrace) {
            Some(self.block()?)
        } else {
            self.eat_punct(Punct::Semi);
            None
        };
        Ok(FunctionDecl {
            modifiers,
            accessor,
            name,
            sig,
            body,
            span: Span::new(start.start, self.prev_span().end),
        })
    }

    fn function_signature(&mut self) -> Result<FunctionSig> {
        self.expect_punct(Punct::LParen)?;
        let mut params = Vec::new();
        while !self.check_punct(Punct::RParen) {
            let start = self.span();
            let is_rest = self.eat_punct(Punct::DotDotDot);
            let name = self.expect_ident()?;
            let type_ref = if self.eat_punct(Punct::Colon) {
                self.type_ref()?
            } else {
                TypeRef::Any
            };
            let default = if self.eat_punct(Punct::Assign) {
                Some(self.expression()?)
            } else {
                None
            };
            params.push(Param {
                name,
                type_ref,
                default,
                is_rest,
                span: Span::new(start.start, self.prev_span().end),
            });
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        self.expect_punct(Punct::RParen)?;
        let return_type = if self.eat_punct(Punct::Colon) {
            self.type_ref()?
        } else {
            TypeRef::Any
        };
        Ok(FunctionSig {
            params,
            return_type,
        })
    }

    fn type_ref(&mut self) -> Result<TypeRef> {
        if self.eat_punct(Punct::Star) {
            return Ok(TypeRef::Any);
        }
        if self.check_kw(Keyword::Void) {
            self.bump();
            return Ok(TypeRef::Void);
        }
        Ok(TypeRef::Named(self.dotted_name()?))
    }

    fn var_decl(&mut self, modifiers: Modifiers) -> Result<Box<VarDecl>> {
        let start = self.span();
        let is_const = self.check_kw(Keyword::Const);
        if is_const {
            self.bump();
        } else {
            self.expect_kw(Keyword::Var)?;
        }
        let name = self.expect_ident()?;
        let type_ref = if self.eat_punct(Punct::Colon) {
            self.type_ref()?
        } else {
            TypeRef::Any
        };
        let init = if self.eat_punct(Punct::Assign) {
            Some(self.expression()?)
        } else {
            None
        };
        Ok(Box::new(VarDecl {
            modifiers,
            is_const,
            name,
            type_ref,
            init,
            span: Span::new(start.start, self.prev_span().end),
        }))
    }

    // ---------------------------------------------------------------- statements

    fn block(&mut self) -> Result<Block> {
        let start = self.expect_punct(Punct::LBrace)?;
        let mut statements = Vec::new();
        while !self.check_punct(Punct::RBrace) {
            if self.at_eof() {
                return Err(Diagnostic::parse("unterminated block", self.span()));
            }
            statements.push(self.statement()?);
        }
        let end = self.expect_punct(Punct::RBrace)?;
        Ok(Block {
            statements,
            span: Span::new(start.start, end.end),
        })
    }

    fn statement(&mut self) -> Result<Stmt> {
        if self.eat_punct(Punct::Semi) {
            return Ok(Stmt::Empty);
        }
        if self.check_punct(Punct::LBrace) {
            return Ok(Stmt::Block(self.block()?));
        }
        if self.check_kw(Keyword::Var) || self.check_kw(Keyword::Const) {
            let decl = self.var_decl(Modifiers::default())?;
            self.eat_punct(Punct::Semi);
            return Ok(Stmt::Var(decl));
        }
        if self.eat_kw(Keyword::Return) {
            let value = if self.check_punct(Punct::Semi) || self.check_punct(Punct::RBrace) {
                None
            } else {
                Some(self.expression()?)
            };
            self.eat_punct(Punct::Semi);
            return Ok(Stmt::Return(value));
        }
        if self.eat_kw(Keyword::If) {
            self.expect_punct(Punct::LParen)?;
            let cond = self.expression()?;
            self.expect_punct(Punct::RParen)?;
            let then = Box::new(self.statement()?);
            let otherwise = if self.eat_kw(Keyword::Else) {
                Some(Box::new(self.statement()?))
            } else {
                None
            };
            return Ok(Stmt::If {
                cond,
                then,
                otherwise,
            });
        }
        if self.eat_kw(Keyword::While) {
            self.expect_punct(Punct::LParen)?;
            let cond = self.expression()?;
            self.expect_punct(Punct::RParen)?;
            let body = Box::new(self.statement()?);
            return Ok(Stmt::While { cond, body });
        }
        if self.check_kw(Keyword::Break) || self.check_kw(Keyword::Continue) {
            let is_break = self.check_kw(Keyword::Break);
            self.bump();
            let label = match self.peek().clone() {
                TokenKind::Ident(name) => {
                    self.bump();
                    Some(name)
                }
                _ => None,
            };
            self.eat_punct(Punct::Semi);
            return Ok(if is_break {
                Stmt::Break(label)
            } else {
                Stmt::Continue(label)
            });
        }

        // Statement forms this phase parses past but will not lower. Each one
        // is skipped by balanced-brace scanning so the rest of the file still
        // parses and the diagnostic points at the construct itself.
        let unsupported = if self.check_kw(Keyword::For) {
            Some("for")
        } else if self.check_kw(Keyword::Do) {
            Some("do/while")
        } else if self.check_kw(Keyword::Switch) {
            Some("switch")
        } else if self.check_kw(Keyword::Try) {
            Some("try/catch")
        } else if self.check_kw(Keyword::Throw) {
            Some("throw")
        } else if self.check_kw(Keyword::With) {
            Some("with")
        } else {
            None
        };
        if let Some(what) = unsupported {
            let span = self.skip_statement()?;
            return Ok(Stmt::Unsupported { what, span });
        }

        let expr = self.expression()?;
        self.eat_punct(Punct::Semi);
        Ok(Stmt::Expr(expr))
    }

    /// Skip a whole statement without interpreting it, balancing brackets, and
    /// return the span covered.
    ///
    /// A statement ends at a `;` outside any bracket, or at the `}` that closes
    /// a brace it opened — except where a keyword continues the same statement
    /// (`do {} while`, `try {} catch/finally`, `if {} else`). Ending at the
    /// closing paren of a `for` or `switch` *header* would leave the body to be
    /// parsed as loose statements, so bracket depth, not the first closer,
    /// decides.
    fn skip_statement(&mut self) -> Result<Span> {
        let start = self.span();
        let mut depth = 0i32;
        loop {
            if self.at_eof() {
                return Err(Diagnostic::parse(
                    "unterminated statement",
                    Span::new(start.start, self.span().end),
                ));
            }
            let here = self.peek().clone();
            match here {
                TokenKind::Punct(Punct::LBrace | Punct::LParen | Punct::LBracket) => {
                    depth += 1;
                    self.bump();
                }
                TokenKind::Punct(p @ (Punct::RBrace | Punct::RParen | Punct::RBracket)) => {
                    if depth == 0 {
                        // The enclosing block's closer — not ours to consume.
                        return Ok(Span::new(start.start, self.prev_span().end));
                    }
                    depth -= 1;
                    self.bump();
                    let continues = matches!(
                        self.peek(),
                        TokenKind::Keyword(
                            Keyword::While | Keyword::Catch | Keyword::Finally | Keyword::Else
                        )
                    );
                    if depth == 0 && p == Punct::RBrace && !continues {
                        return Ok(Span::new(start.start, self.prev_span().end));
                    }
                }
                TokenKind::Punct(Punct::Semi) if depth == 0 => {
                    self.bump();
                    return Ok(Span::new(start.start, self.prev_span().end));
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    // --------------------------------------------------------------- expressions

    pub fn expression(&mut self) -> Result<Expr> {
        self.assignment()
    }

    fn assignment(&mut self) -> Result<Expr> {
        let lhs = self.conditional()?;
        let op = match self.peek() {
            TokenKind::Punct(Punct::Assign) => Some(None),
            TokenKind::Punct(Punct::PlusAssign) => Some(Some(BinOp::Add)),
            TokenKind::Punct(Punct::MinusAssign) => Some(Some(BinOp::Sub)),
            TokenKind::Punct(Punct::StarAssign) => Some(Some(BinOp::Mul)),
            TokenKind::Punct(Punct::SlashAssign) => Some(Some(BinOp::Div)),
            TokenKind::Punct(Punct::PercentAssign) => Some(Some(BinOp::Mod)),
            _ => None,
        };
        let Some(op) = op else { return Ok(lhs) };
        self.bump();
        let value = self.assignment()?;
        let span = Span::new(lhs.span().start, value.span().end);
        Ok(Expr::Assign {
            target: Box::new(lhs),
            op,
            value: Box::new(value),
            span,
        })
    }

    fn conditional(&mut self) -> Result<Expr> {
        let cond = self.binary(0)?;
        if !self.eat_punct(Punct::Question) {
            return Ok(cond);
        }
        let then = self.assignment()?;
        self.expect_punct(Punct::Colon)?;
        let otherwise = self.assignment()?;
        let span = Span::new(cond.span().start, otherwise.span().end);
        Ok(Expr::Conditional {
            cond: Box::new(cond),
            then: Box::new(then),
            otherwise: Box::new(otherwise),
            span,
        })
    }

    /// Binding power of the current token as a binary operator, or `None`.
    /// Higher binds tighter; every operator here is left-associative.
    fn binary_op(&self) -> Option<(BinOp, u8)> {
        let op = match self.peek() {
            TokenKind::Punct(p) => match p {
                Punct::OrOr => (BinOp::Or, 1),
                Punct::AndAnd => (BinOp::And, 2),
                Punct::Pipe => (BinOp::BitOr, 3),
                Punct::Caret => (BinOp::BitXor, 4),
                Punct::Amp => (BinOp::BitAnd, 5),
                Punct::Eq => (BinOp::Eq, 6),
                Punct::Ne => (BinOp::Ne, 6),
                Punct::StrictEq => (BinOp::StrictEq, 6),
                Punct::StrictNe => (BinOp::StrictNe, 6),
                Punct::Lt => (BinOp::Lt, 7),
                Punct::Gt => (BinOp::Gt, 7),
                Punct::Le => (BinOp::Le, 7),
                Punct::Ge => (BinOp::Ge, 7),
                Punct::Shl => (BinOp::Shl, 8),
                Punct::Shr => (BinOp::Shr, 8),
                Punct::UShr => (BinOp::UShr, 8),
                Punct::Plus => (BinOp::Add, 9),
                Punct::Minus => (BinOp::Sub, 9),
                Punct::Star => (BinOp::Mul, 10),
                Punct::Slash => (BinOp::Div, 10),
                Punct::Percent => (BinOp::Mod, 10),
                _ => return None,
            },
            TokenKind::Keyword(k) => match k {
                // `is`/`as`/`instanceof`/`in` sit with the relational operators.
                Keyword::Is => (BinOp::Is, 7),
                Keyword::As => (BinOp::As, 7),
                Keyword::Instanceof => (BinOp::InstanceOf, 7),
                Keyword::In => (BinOp::In, 7),
                _ => return None,
            },
            _ => return None,
        };
        Some(op)
    }

    fn binary(&mut self, min_power: u8) -> Result<Expr> {
        let mut lhs = self.unary()?;
        while let Some((op, power)) = self.binary_op() {
            if power < min_power {
                break;
            }
            self.bump();
            let rhs = self.binary(power + 1)?;
            let span = Span::new(lhs.span().start, rhs.span().end);
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr> {
        let start = self.span();
        let op = match self.peek() {
            TokenKind::Punct(Punct::Not) => Some(UnOp::Not),
            TokenKind::Punct(Punct::Tilde) => Some(UnOp::BitNot),
            TokenKind::Punct(Punct::Minus) => Some(UnOp::Neg),
            TokenKind::Punct(Punct::Plus) => Some(UnOp::Plus),
            TokenKind::Keyword(Keyword::Typeof) => Some(UnOp::TypeOf),
            TokenKind::Keyword(Keyword::Delete) => Some(UnOp::Delete),
            TokenKind::Keyword(Keyword::Void) => Some(UnOp::Void),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let operand = self.unary()?;
            let span = Span::new(start.start, operand.span().end);
            return Ok(Expr::Unary {
                op,
                operand: Box::new(operand),
                span,
            });
        }

        // Prefix `++x` / `--x` desugar to `x = x + 1`, which is what they mean
        // as an expression statement and keeps the AST free of a mutation node
        // that codegen would otherwise have to special-case.
        if self.check_punct(Punct::PlusPlus) || self.check_punct(Punct::MinusMinus) {
            let is_inc = self.check_punct(Punct::PlusPlus);
            self.bump();
            let target = self.unary()?;
            let span = Span::new(start.start, target.span().end);
            return Ok(Expr::Assign {
                target: Box::new(target),
                op: Some(if is_inc { BinOp::Add } else { BinOp::Sub }),
                value: Box::new(Expr::Int(1, span)),
                span,
            });
        }

        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr> {
        let mut expr = self.primary()?;
        loop {
            if self.check_punct(Punct::Dot) {
                self.bump();
                let name = self.expect_ident()?;
                let span = Span::new(expr.span().start, self.prev_span().end);
                expr = Expr::Member {
                    object: Box::new(expr),
                    name,
                    span,
                };
                continue;
            }
            if self.check_punct(Punct::LBracket) {
                self.bump();
                let index = self.expression()?;
                self.expect_punct(Punct::RBracket)?;
                let span = Span::new(expr.span().start, self.prev_span().end);
                expr = Expr::Index {
                    object: Box::new(expr),
                    index: Box::new(index),
                    span,
                };
                continue;
            }
            if self.check_punct(Punct::LParen) {
                let args = self.arguments()?;
                let span = Span::new(expr.span().start, self.prev_span().end);
                expr = Expr::Call {
                    callee: Box::new(expr),
                    args,
                    span,
                };
                continue;
            }
            if self.check_punct(Punct::PlusPlus) || self.check_punct(Punct::MinusMinus) {
                // Postfix in statement position has the same effect as prefix.
                // Codegen rejects it where the *value* is used, rather than
                // silently compiling the wrong one.
                let is_inc = self.check_punct(Punct::PlusPlus);
                self.bump();
                let span = Span::new(expr.span().start, self.prev_span().end);
                expr = Expr::Assign {
                    target: Box::new(expr),
                    op: Some(if is_inc { BinOp::Add } else { BinOp::Sub }),
                    value: Box::new(Expr::Int(1, span)),
                    span,
                };
                continue;
            }
            return Ok(expr);
        }
    }

    fn arguments(&mut self) -> Result<Vec<Expr>> {
        self.expect_punct(Punct::LParen)?;
        let mut args = Vec::new();
        while !self.check_punct(Punct::RParen) {
            args.push(self.expression()?);
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        self.expect_punct(Punct::RParen)?;
        Ok(args)
    }

    fn primary(&mut self) -> Result<Expr> {
        let span = self.span();
        match self.peek().clone() {
            TokenKind::Int(v) => {
                self.bump();
                Ok(Expr::Int(v, span))
            }
            TokenKind::Number(v) => {
                self.bump();
                Ok(Expr::Number(v, span))
            }
            TokenKind::Str(v) => {
                self.bump();
                Ok(Expr::Str(v, span))
            }
            TokenKind::Ident(name) => {
                self.bump();
                Ok(Expr::Ident(name, span))
            }
            TokenKind::Keyword(Keyword::Null) => {
                self.bump();
                Ok(Expr::Null(span))
            }
            TokenKind::Keyword(Keyword::True) => {
                self.bump();
                Ok(Expr::Bool(true, span))
            }
            TokenKind::Keyword(Keyword::False) => {
                self.bump();
                Ok(Expr::Bool(false, span))
            }
            TokenKind::Keyword(Keyword::This) => {
                self.bump();
                Ok(Expr::This(span))
            }
            TokenKind::Keyword(Keyword::Super) => {
                self.bump();
                Ok(Expr::Super(span))
            }
            TokenKind::Keyword(Keyword::New) => {
                self.bump();
                // The callee of `new` is a member expression without its own
                // call: `new a.b.C(x)` constructs `a.b.C`, it does not call it.
                let mut callee = self.primary()?;
                loop {
                    if self.check_punct(Punct::Dot) {
                        self.bump();
                        let name = self.expect_ident()?;
                        let s = Span::new(callee.span().start, self.prev_span().end);
                        callee = Expr::Member {
                            object: Box::new(callee),
                            name,
                            span: s,
                        };
                        continue;
                    }
                    break;
                }
                let args = if self.check_punct(Punct::LParen) {
                    self.arguments()?
                } else {
                    Vec::new()
                };
                Ok(Expr::New {
                    callee: Box::new(callee),
                    args,
                    span: Span::new(span.start, self.prev_span().end),
                })
            }
            TokenKind::Punct(Punct::LParen) => {
                self.bump();
                let inner = self.expression()?;
                self.expect_punct(Punct::RParen)?;
                Ok(inner)
            }
            TokenKind::Punct(Punct::LBracket) => {
                self.bump();
                let mut items = Vec::new();
                while !self.check_punct(Punct::RBracket) {
                    items.push(self.expression()?);
                    if !self.eat_punct(Punct::Comma) {
                        break;
                    }
                }
                self.expect_punct(Punct::RBracket)?;
                Ok(Expr::ArrayLit {
                    items,
                    span: Span::new(span.start, self.prev_span().end),
                })
            }
            other => Err(Diagnostic::parse(
                format!("expected an expression, found {}", other.describe()),
                span,
            )),
        }
    }
}
