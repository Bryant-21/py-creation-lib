"""
Expression evaluator for nif.xml condition strings.

Handles cond, vercond, and calc attributes after token expansion.
Supports: comparisons, logical, bitwise, arithmetic, ternary, #LEN#/#LEN2#.
"""
from __future__ import annotations
import re
from typing import Any


class NifExpr:
    """Compiled expression that evaluates against a context dict."""

    def __init__(self, expr_string: str):
        self.source = expr_string.strip()
        self._tokens = _tokenize(self.source)

    def evaluate(self, context: dict[str, Any]) -> Any:
        """Evaluate expression. Context maps field names to values.
        Returns bool for comparisons, int/float for calculations."""
        parser = _Parser(self._tokens, context)
        result = parser.parse_ternary()
        return result

    def __repr__(self) -> str:
        return f"NifExpr({self.source!r})"


# --- Tokenizer ---

# Token types
_TOKEN_TYPES = [
    ("NUM_HEX", r"0[xX][0-9a-fA-F]+"),
    ("NUM_VER", r"\d+\.\d+\.\d+\.\d+"),  # version literal like 20.2.0.7 — convert to packed int
    ("NUM_FLOAT", r"\d+\.\d+[eE][+\-]?\d+|\d+[eE][+\-]?\d+|\d+\.\d+"),  # float with optional scientific notation
    ("NUM_INT", r"\d+"),                  # plain integer
    ("THEN", r"#THEN#"),
    ("ELSE", r"#ELSE#"),
    ("LEN2", r"#LEN2\[([^\]]+)\]#"),
    ("LEN", r"#LEN\[([^\]]+)\]#"),
    ("OP2", r"==|!=|>=|<=|>>|<<|&&|\|\|"),  # two-char operators
    ("OP1", r"[+\-*/%><!&|^~]"),            # single-char operators
    ("LPAREN", r"\("),
    ("RPAREN", r"\)"),
    ("IDENT", r"[A-Za-z_][A-Za-z0-9_ ]*(?:\\[A-Za-z_][A-Za-z0-9_ ]*)*"),  # field names, may contain spaces and backslash paths
    ("SKIP", r"[ \t]+"),
]

_TOKEN_RE = re.compile("|".join(f"(?P<{name}>{pattern})" for name, pattern in _TOKEN_TYPES))


def _version_str_to_int(ver_str: str) -> int:
    """Convert '20.2.0.7' to packed integer 0x14020007."""
    parts = ver_str.split(".")
    return (int(parts[0]) << 24) | (int(parts[1]) << 16) | (int(parts[2]) << 8) | int(parts[3])


def _tokenize(expr: str) -> list[tuple[str, str]]:
    """Tokenize an expression string into (type, value) pairs."""
    tokens = []
    for m in _TOKEN_RE.finditer(expr):
        kind = m.lastgroup
        value = m.group()
        if kind == "SKIP":
            continue
        if kind == "LEN":
            # Extract field name from #LEN[FieldName]#
            tokens.append(("LEN", re.match(r"#LEN\[([^\]]+)\]#", value).group(1)))
            continue
        if kind == "LEN2":
            tokens.append(("LEN2", re.match(r"#LEN2\[([^\]]+)\]#", value).group(1)))
            continue
        # Version literal: convert to packed int for comparison with Version context
        if kind == "NUM_VER":
            tokens.append(("NUM_INT", str(_version_str_to_int(value))))
            continue
        # Normalize IDENT: strip trailing spaces
        if kind == "IDENT":
            value = value.strip()
        tokens.append((kind, value))
    return tokens


# --- Recursive Descent Parser ---

class _Parser:
    """Recursive descent expression parser/evaluator."""

    def __init__(self, tokens: list[tuple[str, str]], context: dict[str, Any]):
        self.tokens = tokens
        self.pos = 0
        self.ctx = context

    def peek(self) -> tuple[str, str] | None:
        if self.pos < len(self.tokens):
            return self.tokens[self.pos]
        return None

    def advance(self) -> tuple[str, str]:
        tok = self.tokens[self.pos]
        self.pos += 1
        return tok

    def expect(self, kind: str) -> tuple[str, str]:
        tok = self.advance()
        if tok[0] != kind:
            raise ValueError(f"Expected {kind}, got {tok}")
        return tok

    # --- Precedence levels (lowest to highest) ---
    # ternary < logical_or < logical_and < bitwise_or < bitwise_and <
    # equality < relational < shift < additive < multiplicative < unary < primary

    def parse_ternary(self) -> Any:
        """Handle: expr #THEN# expr #ELSE# expr"""
        left = self.parse_logical_or()
        if self.peek() and self.peek()[0] == "THEN":
            self.advance()  # consume #THEN#
            true_val = self.parse_logical_or()
            self.expect("ELSE")
            false_val = self.parse_logical_or()
            return true_val if left else false_val
        return left

    def parse_logical_or(self) -> Any:
        left = self.parse_logical_and()
        while self.peek() and self.peek() == ("OP2", "||"):
            self.advance()
            right = self.parse_logical_and()
            left = bool(left) or bool(right)
        return left

    def parse_logical_and(self) -> Any:
        left = self.parse_bitwise_or()
        while self.peek() and self.peek() == ("OP2", "&&"):
            self.advance()
            right = self.parse_bitwise_or()
            left = bool(left) and bool(right)
        return left

    def parse_bitwise_or(self) -> Any:
        left = self.parse_bitwise_and()
        while self.peek() and self.peek() == ("OP1", "|"):
            self.advance()
            right = self.parse_bitwise_and()
            left = int(left) | int(right)
        return left

    def parse_bitwise_and(self) -> Any:
        left = self.parse_equality()
        while self.peek() and self.peek() == ("OP1", "&"):
            self.advance()
            right = self.parse_equality()
            left = int(left) & int(right)
        return left

    def parse_equality(self) -> Any:
        left = self.parse_relational()
        while self.peek() and self.peek()[0] == "OP2" and self.peek()[1] in ("==", "!="):
            op = self.advance()[1]
            right = self.parse_relational()
            if op == "==":
                left = left == right
            else:
                left = left != right
        return left

    def parse_relational(self) -> Any:
        left = self.parse_shift()
        while self.peek() and (
            (self.peek()[0] == "OP2" and self.peek()[1] in (">=", "<=")) or
            (self.peek()[0] == "OP1" and self.peek()[1] in (">", "<"))
        ):
            op = self.advance()[1]
            right = self.parse_shift()
            if op == ">":
                left = left > right
            elif op == "<":
                left = left < right
            elif op == ">=":
                left = left >= right
            else:
                left = left <= right
        return left

    def parse_shift(self) -> Any:
        left = self.parse_additive()
        while self.peek() and self.peek()[0] == "OP2" and self.peek()[1] in (">>", "<<"):
            op = self.advance()[1]
            right = self.parse_additive()
            if op == ">>":
                left = int(left) >> int(right)
            else:
                left = int(left) << int(right)
        return left

    def parse_additive(self) -> Any:
        left = self.parse_multiplicative()
        while self.peek() and self.peek()[0] == "OP1" and self.peek()[1] in ("+", "-"):
            op = self.advance()[1]
            right = self.parse_multiplicative()
            if op == "+":
                left = left + right
            else:
                left = left - right
        return left

    def parse_multiplicative(self) -> Any:
        left = self.parse_unary()
        while self.peek() and self.peek()[0] == "OP1" and self.peek()[1] in ("*", "/", "%"):
            op = self.advance()[1]
            right = self.parse_unary()
            if op == "*":
                left = left * right
            elif op == "/":
                left = left // right if isinstance(left, int) and isinstance(right, int) else left / right
            else:
                left = left % right
        return left

    def parse_unary(self) -> Any:
        if self.peek() and self.peek() == ("OP1", "!"):
            self.advance()
            val = self.parse_unary()
            return not val
        if self.peek() and self.peek() == ("OP1", "-"):
            self.advance()
            val = self.parse_unary()
            return -val
        return self.parse_primary()

    def parse_primary(self) -> Any:
        tok = self.peek()
        if tok is None:
            return 0

        if tok[0] == "LPAREN":
            self.advance()
            val = self.parse_ternary()
            self.expect("RPAREN")
            return val

        if tok[0] == "NUM_HEX":
            self.advance()
            return int(tok[1], 16)

        if tok[0] == "NUM_FLOAT":
            self.advance()
            return float(tok[1])

        if tok[0] == "NUM_INT":
            self.advance()
            return int(tok[1])

        if tok[0] == "LEN":
            self.advance()
            field_val = self.ctx.get(tok[1], [])
            return len(field_val) if isinstance(field_val, (list, tuple)) else 0

        if tok[0] == "LEN2":
            self.advance()
            field_val = self.ctx.get(tok[1], [])
            if isinstance(field_val, (list, tuple)):
                return sum(len(row) if isinstance(row, (list, tuple)) else 1 for row in field_val)
            return 0

        if tok[0] == "IDENT":
            self.advance()
            # Built-in constants
            if tok[1] == "INFINITY":
                return float("inf")
            return self.ctx.get(tok[1], 0)

        raise ValueError(f"Unexpected token: {tok}")
