use crate::ast::*;
use crate::error::FnvScriptError;
use crate::lexer::{Token, TokenKind, tokenize};

pub fn parse_script(src: &str) -> Result<Script, FnvScriptError> {
    let toks = tokenize(src)?;
    let mut parser = Parser { toks, pos: 0 };
    parser.skip_newlines();
    let name = parser.try_parse_script_name()?;
    let mut variables = Vec::new();
    let mut blocks = Vec::new();

    while !parser.is_eof() {
        parser.skip_newlines();
        if parser.is_eof() {
            break;
        }

        let start = parser.pos;

        if parser.peek_keyword_one_of(&["int", "long", "short", "float", "ref", "string_var"]) {
            variables.push(parser.parse_var_decl()?);
        } else if parser.peek_keyword("Begin") {
            blocks.push(parser.parse_block()?);
        } else {
            blocks.push(Block {
                event: "GameMode".into(),
                args: Vec::new(),
                statements: parser.parse_stmt_list_until(&["End"])?,
            });
        }
        parser.ensure_progress(start, "top-level declaration or block")?;
    }

    Ok(Script {
        name,
        variables,
        blocks,
        script_kind: ScriptKind::Unknown,
    })
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn ensure_progress(&self, start: usize, context: &str) -> Result<(), FnvScriptError> {
        if self.pos > start {
            return Ok(());
        }
        let token = self.peek();
        Err(FnvScriptError::Parse {
            line: token.map_or(1, |token| token.line),
            col: token.map_or(1, |token| token.col),
            msg: format!("parser made no forward progress while parsing {context}"),
        })
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.toks.len()
    }

    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<Token> {
        let token = self.toks.get(self.pos).cloned();
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Newline)) {
            self.pos += 1;
        }
    }

    fn peek_keyword(&self, kw: &str) -> bool {
        matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Keyword(k)) if k.eq_ignore_ascii_case(kw))
    }

    fn peek_keyword_one_of(&self, kws: &[&str]) -> bool {
        kws.iter().any(|kw| self.peek_keyword(kw))
    }

    fn expect_keyword(&mut self, kw: &str) -> Result<(), FnvScriptError> {
        let token = self
            .bump()
            .ok_or_else(|| self.eof_err(format!("expected '{kw}'")))?;
        if let TokenKind::Keyword(found) = &token.kind
            && found.eq_ignore_ascii_case(kw)
        {
            return Ok(());
        }
        Err(FnvScriptError::Parse {
            line: token.line,
            col: token.col,
            msg: format!("expected '{kw}', got {:?}", token.kind),
        })
    }

    fn eof_err(&self, msg: String) -> FnvScriptError {
        match self.toks.last() {
            Some(token) => FnvScriptError::Parse {
                line: token.line,
                col: token.col,
                msg,
            },
            None => FnvScriptError::Parse {
                line: 1,
                col: 1,
                msg,
            },
        }
    }

    fn try_parse_script_name(&mut self) -> Result<Option<String>, FnvScriptError> {
        if !(self.peek_keyword("ScriptName") || self.peek_keyword("scn")) {
            return Ok(None);
        }

        self.bump();
        let token = self
            .bump()
            .ok_or_else(|| self.eof_err("expected script name".into()))?;
        match token.kind {
            TokenKind::Ident(name) | TokenKind::Keyword(name) => Ok(Some(name)),
            _ => Err(FnvScriptError::Parse {
                line: token.line,
                col: token.col,
                msg: "expected script name".into(),
            }),
        }
    }

    fn parse_var_decl(&mut self) -> Result<VarDecl, FnvScriptError> {
        let token = self
            .bump()
            .ok_or_else(|| self.eof_err("expected var type".into()))?;
        let ty = match &token.kind {
            TokenKind::Keyword(k) => match k.to_ascii_lowercase().as_str() {
                "int" => VarType::Int,
                "long" => VarType::Long,
                "short" => VarType::Short,
                "float" => VarType::Float,
                "ref" => VarType::Ref,
                "string_var" => VarType::StringVar,
                other => {
                    return Err(FnvScriptError::Parse {
                        line: token.line,
                        col: token.col,
                        msg: format!("unknown var type {other}"),
                    });
                }
            },
            _ => unreachable!(),
        };
        let name_token = self
            .bump()
            .ok_or_else(|| self.eof_err("expected var name".into()))?;
        let name = match name_token.kind {
            TokenKind::Ident(name) | TokenKind::Keyword(name) => name,
            _ => {
                return Err(FnvScriptError::Parse {
                    line: name_token.line,
                    col: name_token.col,
                    msg: "expected var name".into(),
                });
            }
        };
        let initial = if self.peek_keyword("to") {
            self.bump();
            Some(self.parse_expr()?)
        } else {
            None
        };
        if let Some(token) = self.peek()
            && !matches!(token.kind, TokenKind::Newline)
        {
            return Err(FnvScriptError::Parse {
                line: token.line,
                col: token.col,
                msg: format!(
                    "unexpected token after variable declaration: {:?}",
                    token.kind
                ),
            });
        }
        Ok(VarDecl { name, ty, initial })
    }

    fn parse_block(&mut self) -> Result<Block, FnvScriptError> {
        self.expect_keyword("Begin")?;
        let event_token = self
            .bump()
            .ok_or_else(|| self.eof_err("expected event".into()))?;
        let event = match event_token.kind {
            TokenKind::Ident(name) | TokenKind::Keyword(name) => name,
            _ => {
                return Err(FnvScriptError::Parse {
                    line: event_token.line,
                    col: event_token.col,
                    msg: "expected event name".into(),
                });
            }
        };

        let mut args = Vec::new();
        while let Some(token) = self.peek() {
            if matches!(token.kind, TokenKind::Newline) {
                break;
            }
            let start = self.pos;
            args.push(self.parse_expr()?);
            self.ensure_progress(start, "event argument")?;
        }
        self.skip_newlines();
        let statements = self.parse_stmt_list_until(&["End"])?;
        self.expect_keyword("End")?;

        Ok(Block {
            event,
            args,
            statements,
        })
    }

    fn parse_stmt_list_until(&mut self, terminators: &[&str]) -> Result<Vec<Stmt>, FnvScriptError> {
        let mut statements = Vec::new();
        loop {
            self.skip_newlines();
            if self.is_eof() {
                break;
            }
            if terminators.iter().any(|term| self.peek_keyword(term)) {
                break;
            }
            let start = self.pos;
            if self.peek_keyword("Set") {
                statements.push(self.parse_set()?);
                self.ensure_progress(start, "set statement")?;
                continue;
            }
            if self.peek_keyword("if") {
                statements.push(self.parse_if()?);
                self.ensure_progress(start, "if statement")?;
                continue;
            }
            if self.peek_keyword("Return") {
                self.bump();
                statements.push(Stmt::Return);
                self.ensure_progress(start, "return statement")?;
                continue;
            }

            let call = self.parse_call_stmt()?;
            statements.push(Stmt::Call(call));
            self.ensure_progress(start, "call statement")?;
        }
        Ok(statements)
    }

    fn parse_set(&mut self) -> Result<Stmt, FnvScriptError> {
        self.expect_keyword("Set")?;
        let target = self.parse_lvalue()?;
        self.expect_keyword("to")?;
        let value = self.parse_expr()?;
        Ok(Stmt::Set { target, value })
    }

    fn parse_if(&mut self) -> Result<Stmt, FnvScriptError> {
        let if_line = self.peek().map_or(1, |token| token.line);
        let if_col = self.peek().map_or(1, |token| token.col);
        self.expect_keyword("if")?;
        let cond = self.parse_expr()?;
        self.skip_newlines();
        let then_branch = self.parse_stmt_list_until(&["elseif", "else", "endif"])?;
        let mut elif_branches = Vec::new();

        while self.peek_keyword("elseif") {
            self.bump();
            let elif_cond = self.parse_expr()?;
            self.skip_newlines();
            let body = self.parse_stmt_list_until(&["elseif", "else", "endif"])?;
            elif_branches.push((elif_cond, body));
        }

        let mut else_branch = if self.peek_keyword("else") {
            self.bump();
            if matches!(
                self.peek().map(|token| &token.kind),
                Some(TokenKind::Punct('('))
            ) {
                let elif_cond = self.parse_expr()?;
                self.expect_line_end("after else(condition)")?;
                let body = self.parse_stmt_list_until(&["endif"])?;
                elif_branches.push((elif_cond, body));
                Vec::new()
            } else {
                self.skip_newlines();
                self.parse_stmt_list_until(&["endif"])?
            }
        } else {
            Vec::new()
        };

        let endif_col = self.peek().map_or(1, |token| token.col);
        self.expect_keyword("endif").map_err(|error| match error {
            FnvScriptError::Parse { line, col, msg } => FnvScriptError::Parse {
                line,
                col,
                msg: format!("{msg} for if starting on line {if_line}"),
            },
            other => other,
        })?;

        if elif_branches.is_empty() && else_branch.is_empty() {
            // Some GECK sources close an If immediately before a same-level ElseIf.
            let checkpoint = self.pos;
            self.skip_newlines();
            if self.peek_keyword_at_either_col("elseif", if_col, endif_col) {
                while self.peek_keyword_at_either_col("elseif", if_col, endif_col) {
                    self.bump();
                    let elif_cond = self.parse_expr()?;
                    self.skip_newlines();
                    let body = self.parse_stmt_list_until(&["elseif", "else", "endif"])?;
                    elif_branches.push((elif_cond, body));
                }
                if self.peek_keyword_at_either_col("else", if_col, endif_col) {
                    self.bump();
                    self.skip_newlines();
                    else_branch = self.parse_stmt_list_until(&["endif"])?;
                }
                self.expect_keyword("endif")?;
            } else {
                self.pos = checkpoint;
            }
        }

        Ok(Stmt::If {
            cond,
            then_branch,
            elif_branches,
            else_branch,
        })
    }

    fn expect_line_end(&mut self, context: &str) -> Result<(), FnvScriptError> {
        match self.peek() {
            Some(Token {
                kind: TokenKind::Newline,
                ..
            }) => {
                self.skip_newlines();
                Ok(())
            }
            Some(token) => Err(FnvScriptError::Parse {
                line: token.line,
                col: token.col,
                msg: format!("expected newline {context}, got {:?}", token.kind),
            }),
            None => Err(self.eof_err(format!("expected newline {context}"))),
        }
    }

    fn parse_lvalue(&mut self) -> Result<LValue, FnvScriptError> {
        let token = self
            .bump()
            .ok_or_else(|| self.eof_err("expected lvalue".into()))?;
        let name = match token.kind {
            TokenKind::Ident(name) | TokenKind::Keyword(name) => name,
            _ => {
                return Err(FnvScriptError::Parse {
                    line: token.line,
                    col: token.col,
                    msg: "expected identifier".into(),
                });
            }
        };

        if matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Punct('.'))) {
            self.bump();
            let member_token = self
                .bump()
                .ok_or_else(|| self.eof_err("expected member".into()))?;
            let member = match member_token.kind {
                TokenKind::Ident(name) | TokenKind::Keyword(name) => name,
                _ => {
                    return Err(FnvScriptError::Parse {
                        line: member_token.line,
                        col: member_token.col,
                        msg: "expected member".into(),
                    });
                }
            };
            return Ok(LValue::Member {
                receiver: Expr::Ident(name),
                name: member,
            });
        }

        Ok(LValue::Var(name))
    }

    fn parse_call_stmt(&mut self) -> Result<FunctionCall, FnvScriptError> {
        let head = self
            .bump()
            .ok_or_else(|| self.eof_err("expected function call".into()))?;
        let mut name = match head.kind {
            TokenKind::Ident(name) | TokenKind::Keyword(name) => name,
            _ => {
                return Err(FnvScriptError::Parse {
                    line: head.line,
                    col: head.col,
                    msg: "expected function name".into(),
                });
            }
        };

        let mut receiver = None;
        if matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Punct('.'))) {
            self.bump();
            let member_token = self
                .bump()
                .ok_or_else(|| self.eof_err("expected member".into()))?;
            let member = match member_token.kind {
                TokenKind::Ident(name) | TokenKind::Keyword(name) => name,
                _ => {
                    return Err(FnvScriptError::Parse {
                        line: member_token.line,
                        col: member_token.col,
                        msg: "expected member".into(),
                    });
                }
            };
            receiver = Some(Box::new(Expr::Ident(name)));
            name = member;
        }

        let mut args = Vec::new();
        while let Some(token) = self.peek() {
            if matches!(token.kind, TokenKind::Newline) {
                break;
            }
            let start = self.pos;
            args.push(self.parse_expr()?);
            self.ensure_progress(start, "call argument")?;
        }

        Ok(FunctionCall {
            name,
            receiver,
            args,
        })
    }

    fn parse_expr(&mut self) -> Result<Expr, FnvScriptError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, FnvScriptError> {
        let mut lhs = self.parse_and()?;
        while matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Op(op)) if op == "||") {
            self.bump();
            let rhs = self.parse_and()?;
            lhs = Expr::BinOp {
                op: BinOp::Or,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Expr, FnvScriptError> {
        let mut lhs = self.parse_cmp()?;
        while matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Op(op)) if op == "&&") {
            self.bump();
            let rhs = self.parse_cmp()?;
            lhs = Expr::BinOp {
                op: BinOp::And,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_cmp(&mut self) -> Result<Expr, FnvScriptError> {
        let lhs = self.parse_add()?;
        let op = match self.peek().map(|t| &t.kind) {
            Some(TokenKind::Op(op)) => match op.as_str() {
                "==" => Some(BinOp::Eq),
                "!=" => Some(BinOp::Ne),
                "<" => Some(BinOp::Lt),
                "<=" => Some(BinOp::Le),
                ">" => Some(BinOp::Gt),
                ">=" => Some(BinOp::Ge),
                _ => None,
            },
            _ => None,
        };

        if let Some(op) = op {
            self.bump();
            let rhs = self.parse_add()?;
            return Ok(Expr::BinOp {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            });
        }

        Ok(lhs)
    }

    fn parse_add(&mut self) -> Result<Expr, FnvScriptError> {
        let mut lhs = self.parse_mul()?;
        loop {
            let op = match self.peek().map(|t| &t.kind) {
                Some(TokenKind::Op(op)) => match op.as_str() {
                    "+" => Some(BinOp::Add),
                    "-" => Some(BinOp::Sub),
                    _ => None,
                },
                _ => None,
            };
            let Some(op) = op else {
                break;
            };
            self.bump();
            let rhs = self.parse_mul()?;
            lhs = Expr::BinOp {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_mul(&mut self) -> Result<Expr, FnvScriptError> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek().map(|t| &t.kind) {
                Some(TokenKind::Op(op)) => match op.as_str() {
                    "*" => Some(BinOp::Mul),
                    "/" => Some(BinOp::Div),
                    "%" => Some(BinOp::Mod),
                    _ => None,
                },
                _ => None,
            };
            let Some(op) = op else {
                break;
            };
            self.bump();
            let rhs = self.parse_unary()?;
            lhs = Expr::BinOp {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, FnvScriptError> {
        if matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Op(op)) if op == "-") {
            self.bump();
            let operand = self.parse_unary()?;
            return Ok(Expr::UnaryOp {
                op: UnaryOp::Neg,
                operand: Box::new(operand),
            });
        }
        if matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Op(op)) if op == "!") {
            self.bump();
            let operand = self.parse_unary()?;
            return Ok(Expr::UnaryOp {
                op: UnaryOp::Not,
                operand: Box::new(operand),
            });
        }
        self.parse_atom()
    }

    fn parse_atom(&mut self) -> Result<Expr, FnvScriptError> {
        let token = self
            .bump()
            .ok_or_else(|| self.eof_err("expected expression".into()))?;
        match token.kind {
            TokenKind::IntLit(value) => Ok(Expr::Int(value)),
            TokenKind::FloatLit(value) => Ok(Expr::Float(value)),
            TokenKind::StringLit(value) => Ok(Expr::String(value)),
            TokenKind::Ident(name) | TokenKind::Keyword(name) => {
                let mut receiver = None;
                let mut current_name = name;
                if matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Punct('.'))) {
                    self.bump();
                    let member_token = self
                        .bump()
                        .ok_or_else(|| self.eof_err("expected member".into()))?;
                    let member = match member_token.kind {
                        TokenKind::Ident(name) | TokenKind::Keyword(name) => name,
                        _ => {
                            return Err(FnvScriptError::Parse {
                                line: member_token.line,
                                col: member_token.col,
                                msg: "expected member".into(),
                            });
                        }
                    };
                    receiver = Some(Box::new(Expr::Ident(current_name)));
                    current_name = member;
                }
                if matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Punct('('))) {
                    self.bump();
                    let mut args = Vec::new();
                    if !matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Punct(')'))) {
                        loop {
                            let start = self.pos;
                            args.push(self.parse_expr()?);
                            self.ensure_progress(start, "parenthesized call argument")?;
                            if matches!(self.peek().map(|t| &t.kind), Some(TokenKind::Punct(','))) {
                                self.bump();
                                continue;
                            }
                            break;
                        }
                    }
                    let closing = self
                        .bump()
                        .ok_or_else(|| self.eof_err("expected ')'".into()))?;
                    if !matches!(closing.kind, TokenKind::Punct(')')) {
                        return Err(FnvScriptError::Parse {
                            line: closing.line,
                            col: closing.col,
                            msg: "expected ')'".into(),
                        });
                    }
                    return Ok(Expr::Call(FunctionCall {
                        name: current_name,
                        receiver,
                        args,
                    }));
                }
                if looks_like_prefix_function(&current_name) && self.prefix_arg_starts_here() {
                    let mut args = Vec::new();
                    while self.prefix_arg_starts_here() {
                        args.push(self.parse_unary()?);
                    }
                    return Ok(Expr::Call(FunctionCall {
                        name: current_name,
                        receiver,
                        args,
                    }));
                }
                if let Some(receiver) = receiver {
                    return Ok(Expr::Member {
                        receiver,
                        name: current_name,
                    });
                }
                Ok(Expr::Ident(current_name))
            }
            TokenKind::Punct('(') => {
                let expr = self.parse_expr()?;
                let closing = self
                    .bump()
                    .ok_or_else(|| self.eof_err("expected ')'".into()))?;
                if !matches!(closing.kind, TokenKind::Punct(')')) {
                    return Err(FnvScriptError::Parse {
                        line: closing.line,
                        col: closing.col,
                        msg: "expected ')'".into(),
                    });
                }
                Ok(expr)
            }
            other => Err(FnvScriptError::Parse {
                line: token.line,
                col: token.col,
                msg: format!("unexpected token {other:?}"),
            }),
        }
    }

    fn prefix_arg_starts_here(&self) -> bool {
        match self.peek().map(|token| &token.kind) {
            Some(TokenKind::Ident(_))
            | Some(TokenKind::IntLit(_))
            | Some(TokenKind::FloatLit(_))
            | Some(TokenKind::StringLit(_))
            | Some(TokenKind::Punct('(')) => true,
            Some(TokenKind::Keyword(keyword)) => !matches!(
                keyword.to_ascii_lowercase().as_str(),
                "elseif" | "else" | "endif" | "end" | "to" | "begin" | "set" | "return"
            ),
            _ => false,
        }
    }

    fn peek_keyword_at_col(&self, keyword: &str, col: usize) -> bool {
        self.peek_keyword(keyword) && self.peek().is_some_and(|token| token.col == col)
    }

    fn peek_keyword_at_either_col(&self, keyword: &str, first: usize, second: usize) -> bool {
        self.peek_keyword_at_col(keyword, first) || self.peek_keyword_at_col(keyword, second)
    }
}

fn looks_like_prefix_function(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    normalized.starts_with("get") || normalized.starts_with("is") || normalized.starts_with("has")
}
