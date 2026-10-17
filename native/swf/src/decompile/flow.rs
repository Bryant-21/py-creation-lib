//! Method bodies: basic blocks, symbolic stack execution, then reduction of the block graph
//! into expressions (&&, ||, ?:), compound conditions and structured statements.

use super::Abc;
use super::expr::{BinOp, Catch, Expr, Key, Lit, Stmt, UnOp, invert, negate, ternary};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use swf::avm2::types::*;

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, PartialEq)]
pub enum Term {
    Goto(usize),
    /// Jump to the first block when the condition holds, else to the second.
    Cond(Expr, usize, usize),
    Switch(Expr, Vec<usize>, usize),
    Return(Option<Expr>),
    Throw(Expr),
    End,
}

impl Term {
    fn succs(&self) -> Vec<usize> {
        match self {
            Term::Goto(t) => vec![*t],
            Term::Cond(_, t, f) => vec![*t, *f],
            Term::Switch(_, cases, default) => {
                let mut all = cases.clone();
                all.push(*default);
                all
            }
            _ => vec![],
        }
    }

    fn substitute(&self, values: &[Expr]) -> Term {
        match self {
            Term::Cond(c, t, f) => Term::Cond(c.substitute(values), *t, *f),
            Term::Switch(e, cases, d) => Term::Switch(e.substitute(values), cases.clone(), *d),
            Term::Return(Some(e)) => Term::Return(Some(e.substitute(values))),
            Term::Throw(e) => Term::Throw(e.substitute(values)),
            other => other.clone(),
        }
    }

    fn retarget(&mut self, from: usize, to: usize) {
        let swap = |b: &mut usize| {
            if *b == from {
                *b = to;
            }
        };
        match self {
            Term::Goto(t) => swap(t),
            Term::Cond(_, t, f) => {
                swap(t);
                swap(f);
            }
            Term::Switch(_, cases, d) => {
                cases.iter_mut().for_each(swap);
                swap(d);
            }
            _ => {}
        }
    }
}

fn substitute_stmt(s: &Stmt, values: &[Expr]) -> Stmt {
    match s {
        Stmt::Expr(e) => Stmt::Expr(e.substitute(values)),
        Stmt::Return(Some(e)) => Stmt::Return(Some(e.substitute(values))),
        Stmt::Throw(e) => Stmt::Throw(e.substitute(values)),
        other => other.clone(),
    }
}

pub struct Block {
    start: usize,
    end: usize,
    offset: usize,
    depth: usize,
    reached: bool,
    alive: bool,
    handler: Option<usize>,
    stmts: Vec<Stmt>,
    exit: Vec<Expr>,
    term: Term,
}

#[derive(Clone)]
struct Val {
    e: Expr,
    dup: u32,
    inc_of: u32,
    /// The compiler temporary this value was also stored in.
    temp: u32,
}

impl Val {
    fn new(e: Expr) -> Self {
        Self { e, dup: 0, inc_of: 0, temp: 0 }
    }
}

/// Compiler temporaries (registers it `kill`s) within one block: their stores are statements
/// until a read shows the value can be used in place.
#[derive(Default)]
struct Temps {
    held: HashMap<u32, Held>,
    /// Statements found redundant; dropped when the block is done.
    removed: HashSet<usize>,
    /// Statements that only store a side-effect-free value.
    plain: HashSet<usize>,
}

struct Held {
    at: usize,
    value: Val,
    /// The property a chained assignment set from the temporary: `target = temp = value`.
    target: Option<Expr>,
}

impl Temps {
    fn quiet_since(&self, at: usize, len: usize) -> bool {
        (at + 1..len).all(|j| self.removed.contains(&j) || self.plain.contains(&j))
    }

    fn read(&mut self, index: u32, stmts: &mut [Stmt]) -> Option<Val> {
        let h = self.held.get(&index)?;
        let at = h.at;
        if !self.quiet_since(at, stmts.len()) {
            // Something happened in between: the register keeps its store.
            self.removed.remove(&at);
            self.held.remove(&index);
            return None;
        }
        let h = self.held.get_mut(&index)?;
        self.removed.insert(at);
        let Some(target) = h.target.take() else { return Some(h.value.clone()) };
        let value = h.value.e.clone();
        if value.has_effects() {
            self.held.remove(&index);
        } else {
            stmts[at] = Stmt::Expr(Expr::Assign(Box::new(Expr::Local(index)), Box::new(value.clone())));
            self.plain.insert(at);
        }
        Some(Val::new(Expr::Assign(Box::new(target), Box::new(value))))
    }

    fn kill(&mut self, index: u32, stmts: &mut [Stmt]) {
        let Some(h) = self.held.remove(&index) else { return };
        match h.target {
            Some(target) => {
                stmts[h.at] = Stmt::Expr(Expr::Assign(Box::new(target), Box::new(h.value.e)));
                self.plain.remove(&h.at);
            }
            None => {
                self.removed.insert(h.at);
            }
        }
    }
}

struct Frame {
    brk: Option<usize>,
    cont: Option<usize>,
    label: String,
    used: bool,
}

pub struct Body<'a> {
    abc: &'a Abc<'a>,
    body: &'a MethodBody,
    ops: Vec<(usize, Op)>,
    pub names: RefCell<HashMap<u32, String>>,
    params: u32,
    blocks: Vec<Block>,
    block_of: Vec<usize>,
    serial: u32,
    // structuring state
    done: Vec<bool>,
    entered: HashSet<usize>,
    loops: HashMap<usize, BTreeSet<usize>>,
    tries_done: HashSet<(u32, u32)>,
    labels: usize,
    /// Closures in creation order, with the name a named function expression gave them.
    pub closures: RefCell<Vec<(Index<Method>, Option<String>)>>,
    /// What `getlocal 0` is: `this`, or the class in static code.
    pub this: Expr,
    /// Stores (by instruction) whose value is read straight back and returned.
    single: HashSet<usize>,
    /// The register a method keeps its activation in, for code (catch blocks) that reloads it.
    activation: Option<u32>,
    /// Declared local types, from the conversion before each store.
    pub types: HashMap<u32, String>,
    /// The plain type of every value stored in each register (`None` when unknown).
    pub stored: HashMap<u32, Vec<Option<String>>>,
    temps: HashSet<u32>,
    /// Every register the compiler kills.
    killed: HashSet<u32>,
    /// Conditions joined from separate jumps, which compilers emit for nested ifs.
    compounds: Vec<Expr>,
}

fn branch_target(next: usize, relative: i32) -> i64 {
    next as i64 + relative as i64
}

impl<'a> Body<'a> {
    pub fn new(abc: &'a Abc<'a>, body: &'a MethodBody, ops: Vec<(usize, Op)>, names: HashMap<u32, String>, params: u32) -> Self {
        Self {
            abc,
            body,
            ops,
            names: RefCell::new(names),
            params,
            blocks: Vec::new(),
            block_of: Vec::new(),
            serial: 0,
            done: Vec::new(),
            entered: HashSet::new(),
            loops: HashMap::new(),
            tries_done: HashSet::new(),
            labels: 0,
            closures: RefCell::new(Vec::new()),
            this: Expr::This,
            single: HashSet::new(),
            activation: None,
            types: HashMap::new(),
            stored: HashMap::new(),
            temps: HashSet::new(),
            killed: HashSet::new(),
            compounds: Vec::new(),
        }
    }

    /// Registers the compiler kills whose every read follows a store in the same block: their
    /// values never cross blocks, so reads can take the stored value in place.
    fn find_temps(&mut self) {
        let mut killed = HashSet::new();
        let mut excluded = HashSet::new();
        let mut stored = HashSet::new();
        for (i, (_, op)) in self.ops.iter().enumerate() {
            if i > 0 && self.block_of[i] != self.block_of[i - 1] {
                stored.clear();
            }
            match op {
                Op::Kill { index } => {
                    killed.insert(*index);
                    stored.remove(index);
                }
                Op::SetLocal { index } => {
                    stored.insert(*index);
                }
                Op::GetLocal { index } if !stored.contains(index) => {
                    excluded.insert(*index);
                }
                Op::HasNext2 { object_register, index_register } => {
                    excluded.extend([*object_register, *index_register]);
                }
                Op::IncLocal { index } | Op::IncLocalI { index } | Op::DecLocal { index } | Op::DecLocalI { index } => {
                    excluded.insert(*index);
                }
                _ => {}
            }
        }
        self.temps = killed.iter().copied().filter(|r| *r > self.params && !excluded.contains(r)).collect();
        self.killed = killed;
        // The register the activation lives in, which catch blocks read before the prologue runs.
        for w in self.ops.windows(3) {
            match (&w[0].1, &w[1].1, &w[2].1) {
                (Op::NewActivation, Op::Dup, Op::SetLocal { index }) | (Op::NewActivation, Op::SetLocal { index }, _) => {
                    self.activation = Some(*index);
                }
                _ => {}
            }
        }
        // Stored, read back and returned: the store is dead once the method returns.
        self.single = self
            .ops
            .windows(3)
            .enumerate()
            .filter_map(|(i, w)| match (&w[0].1, &w[1].1, &w[2].1) {
                (Op::SetLocal { index: a }, Op::GetLocal { index: b }, Op::ReturnValue)
                    if a == b && self.block_of[i] == self.block_of[i + 2] =>
                {
                    Some(i)
                }
                _ => None,
            })
            .filter(|i| matches!(self.ops[*i].1, Op::SetLocal { index } if index > self.params))
            .collect();
    }

    fn next_offset(&self, i: usize) -> usize {
        self.ops.get(i + 1).map_or(self.body.code.len(), |o| o.0)
    }

    fn index_at(&self, offset: i64) -> Result<usize> {
        self.ops
            .binary_search_by_key(&offset, |o| o.0 as i64)
            .map_err(|_| format!("branch into the middle of an instruction at byte {offset}"))
    }

    fn targets(&self, i: usize) -> Result<Vec<usize>> {
        let (offset, op) = &self.ops[i];
        let next = self.next_offset(i);
        Ok(match op {
            Op::Jump { offset: r }
            | Op::IfTrue { offset: r }
            | Op::IfFalse { offset: r }
            | Op::IfEq { offset: r }
            | Op::IfNe { offset: r }
            | Op::IfLt { offset: r }
            | Op::IfLe { offset: r }
            | Op::IfGt { offset: r }
            | Op::IfGe { offset: r }
            | Op::IfNlt { offset: r }
            | Op::IfNle { offset: r }
            | Op::IfNgt { offset: r }
            | Op::IfNge { offset: r }
            | Op::IfStrictEq { offset: r }
            | Op::IfStrictNe { offset: r } => vec![self.index_at(branch_target(next, *r))?],
            // lookupswitch offsets are relative to the switch instruction itself.
            Op::LookupSwitch(table) => {
                let mut all = vec![self.index_at(*offset as i64 + table.default_offset as i64)?];
                for c in table.case_offsets.iter() {
                    all.push(self.index_at(*offset as i64 + *c as i64)?);
                }
                all
            }
            _ => vec![],
        })
    }

    fn ends_block(op: &Op) -> bool {
        matches!(
            op,
            Op::Jump { .. }
                | Op::IfTrue { .. }
                | Op::IfFalse { .. }
                | Op::IfEq { .. }
                | Op::IfNe { .. }
                | Op::IfLt { .. }
                | Op::IfLe { .. }
                | Op::IfGt { .. }
                | Op::IfGe { .. }
                | Op::IfNlt { .. }
                | Op::IfNle { .. }
                | Op::IfNgt { .. }
                | Op::IfNge { .. }
                | Op::IfStrictEq { .. }
                | Op::IfStrictNe { .. }
                | Op::LookupSwitch(_)
                | Op::ReturnValue
                | Op::ReturnVoid
                | Op::Throw
        )
    }

    /// Compilers leave dead jumps after a return, sometimes into the middle of an instruction;
    /// only code that can run defines blocks.
    fn reachable_ops(&self) -> Result<Vec<bool>> {
        let mut reachable = vec![false; self.ops.len()];
        let mut work = vec![0usize];
        for e in &self.body.exceptions {
            work.push(self.index_at(e.target_offset as i64)?);
        }
        while let Some(i) = work.pop() {
            if i >= self.ops.len() || reachable[i] {
                continue;
            }
            reachable[i] = true;
            work.extend(self.targets(i)?);
            if !matches!(self.ops[i].1, Op::Jump { .. } | Op::LookupSwitch(_) | Op::ReturnValue | Op::ReturnVoid | Op::Throw) {
                work.push(i + 1);
            }
        }
        Ok(reachable)
    }

    fn build_blocks(&mut self) -> Result<()> {
        if self.ops.is_empty() {
            return Err("empty method body".into());
        }
        let reachable = self.reachable_ops()?;
        let mut leaders = BTreeSet::from([0usize]);
        for i in 0..self.ops.len() {
            if !reachable[i] {
                // Each dead instruction stands alone, so no dead jump is ever simulated.
                leaders.insert(i);
                leaders.insert(i + 1);
                continue;
            }
            for t in self.targets(i)? {
                leaders.insert(t);
            }
            if Self::ends_block(&self.ops[i].1) {
                leaders.insert(i + 1);
            }
        }
        let mut handlers = HashMap::new();
        for (n, e) in self.body.exceptions.iter().enumerate() {
            for offset in [e.from_offset, e.to_offset] {
                if let Ok(i) = self.index_at(offset as i64) {
                    leaders.insert(i);
                }
            }
            let target = self.index_at(e.target_offset as i64)?;
            leaders.insert(target);
            handlers.entry(target).or_insert(n);
        }
        leaders.retain(|&i| i < self.ops.len());
        let starts: Vec<usize> = leaders.into_iter().collect();
        self.block_of = vec![0; self.ops.len()];
        for (b, &start) in starts.iter().enumerate() {
            let end = starts.get(b + 1).copied().unwrap_or(self.ops.len());
            for i in start..end {
                self.block_of[i] = b;
            }
            self.blocks.push(Block {
                start,
                end,
                offset: self.ops[start].0,
                depth: 0,
                reached: false,
                alive: false,
                handler: handlers.get(&start).copied(),
                stmts: Vec::new(),
                exit: Vec::new(),
                term: Term::End,
            });
        }
        Ok(())
    }

    /// Simulates every reachable block once, with placeholders for values it finds on the stack.
    fn simulate_all(&mut self) -> Result<()> {
        let mut work: Vec<(usize, usize, Vec<Expr>)> = vec![(0, 0, vec![])];
        for b in 0..self.blocks.len() {
            if self.blocks[b].handler.is_some() {
                work.push((b, 0, vec![]));
            }
        }
        while let Some((b, depth, scopes)) = work.pop() {
            if self.blocks[b].reached {
                if self.blocks[b].depth != depth {
                    return Err(format!("stack depth differs at byte {}", self.blocks[b].offset));
                }
                continue;
            }
            self.blocks[b].reached = true;
            self.blocks[b].alive = true;
            self.blocks[b].depth = depth;
            let stack = match self.blocks[b].handler {
                Some(n) => vec![Val::new(Expr::Name(self.catch_name(n).0))],
                None => (0..depth).map(|i| Val::new(Expr::In(i))).collect(),
            };
            let exit_scopes = self.simulate(b, stack, scopes)?;
            let exit_depth = self.blocks[b].exit.len();
            for s in self.blocks[b].term.succs() {
                work.push((s, exit_depth, exit_scopes.clone()));
            }
        }
        Ok(())
    }

    fn catch_name(&self, n: usize) -> (String, String) {
        let e = &self.body.exceptions[n];
        let name = self.abc.simple(e.variable_name);
        let name = if name == "*" || name.is_empty() { "e".to_string() } else { name };
        (name, self.abc.type_name(e.type_name))
    }

    fn slot_name(&self, slot: u32) -> Option<String> {
        self.body.traits.iter().find_map(|t| match &t.kind {
            TraitKind::Slot { slot_id, .. } | TraitKind::Const { slot_id, .. } if *slot_id == slot => {
                Some(self.abc.simple(t.name))
            }
            _ => None,
        })
    }

    fn key(&self, index: Index<Multiname>, stack: &mut Vec<Val>) -> Result<Key> {
        let parts = self.abc.key_parts(index);
        let name = if parts.runtime_name { Some(pop(stack)?.e) } else { None };
        if parts.runtime_ns {
            pop(stack)?;
        }
        Ok(match name {
            Some(e) => Key::Index(Box::new(e)),
            None if parts.attribute => Key::Attr(parts.name),
            None => Key::Name(parts.name),
        })
    }

    fn simulate(&mut self, b: usize, mut stack: Vec<Val>, mut scopes: Vec<Expr>) -> Result<Vec<Expr>> {
        let pool = &self.abc.abc.constant_pool;
        let mut stmts = Vec::new();
        let mut temps = Temps::default();
        let mut term = None;
        let (start, end) = (self.blocks[b].start, self.blocks[b].end);
        let mut skip = false;
        for i in start..end {
            if std::mem::take(&mut skip) {
                continue;
            }
            let op = self.ops[i].1.clone();
            let next_block = |body: &Self, target: i64| -> Result<usize> { Ok(body.block_of[body.index_at(target)?]) };
            let next = self.next_offset(i);
            let fall = if end < self.ops.len() { Some(self.block_of[end]) } else { None };
            macro_rules! push {
                ($e:expr) => {
                    stack.push(Val::new($e))
                };
            }
            macro_rules! binary {
                ($op:expr) => {{
                    let r = pop(&mut stack)?.e;
                    let l = pop(&mut stack)?.e;
                    push!(Expr::bin($op, l, r));
                }};
            }
            macro_rules! cond {
                ($relative:expr, $make:expr) => {{
                    let target = next_block(self, branch_target(next, $relative))?;
                    let c: Expr = $make;
                    let fall = fall.ok_or("conditional jump at end of method")?;
                    // Compile-time constants decide the branch.
                    term = Some(match c {
                        Expr::Lit(Lit::True) => Term::Goto(target),
                        Expr::Lit(Lit::False) => Term::Goto(fall),
                        c => Term::Cond(c, target, fall),
                    });
                }};
            }
            macro_rules! compare {
                ($relative:expr, $op:expr, $negated:expr) => {{
                    let r = pop(&mut stack)?.e;
                    let l = pop(&mut stack)?.e;
                    let c = Expr::bin($op, l, r);
                    cond!($relative, if $negated { Expr::Unary(UnOp::Not, Box::new(c)) } else { c })
                }};
            }
            match op {
                Op::Nop | Op::Label | Op::Bkpt | Op::BkptLine { .. } | Op::Timestamp => {}
                Op::DebugLine { .. } | Op::DebugFile { .. } | Op::Debug { .. } => {}
                Op::Kill { index } => temps.kill(index, &mut stmts),
                Op::PushByte { value } => push!(Expr::int(value as i8 as i64)),
                Op::PushShort { value } => push!(Expr::int(value as i64)),
                Op::PushInt { value } => push!(Expr::int(pooled(&pool.ints, value.0).map_or(0, |v| v as i64))),
                Op::PushUint { value } => push!(Expr::int(pooled(&pool.uints, value.0).map_or(0, |v| v as i64))),
                Op::PushDouble { value } => push!(Expr::Lit(Lit::Num(pooled(&pool.doubles, value.0).unwrap_or(f64::NAN)))),
                Op::PushString { value } => push!(Expr::Lit(Lit::Str(self.abc.string(value)))),
                Op::PushTrue => push!(Expr::Lit(Lit::True)),
                Op::PushFalse => push!(Expr::Lit(Lit::False)),
                Op::PushNull => push!(Expr::Lit(Lit::Null)),
                Op::PushUndefined => push!(Expr::Lit(Lit::Undefined)),
                Op::PushNaN => push!(Expr::Lit(Lit::NaN)),
                Op::PushNamespace { value } => push!(Expr::Lit(Lit::Str(self.abc.namespace_uri(value)))),
                Op::GetLocal { index } => match temps.read(index, &mut stmts) {
                    Some(v) => stack.push(v),
                    None if Some(index) == self.activation => push!(Expr::Activation),
                    None => push!(if index == 0 { self.this.clone() } else { Expr::Local(index) }),
                },
                Op::SetLocal { .. } if self.single.contains(&i) => {
                    // Stored, read straight back and returned: the value stays where it is.
                    skip = true;
                }
                Op::SetLocal { index } => {
                    let v = pop(&mut stack)?;
                    self.set_local(index, v, &mut stack, &mut stmts, &mut temps);
                }
                Op::IncLocal { index } | Op::IncLocalI { index } => stmts.push(Stmt::Expr(Expr::Update {
                    target: Box::new(Expr::Local(index)),
                    inc: true,
                    prefix: false,
                })),
                Op::DecLocal { index } | Op::DecLocalI { index } => stmts.push(Stmt::Expr(Expr::Update {
                    target: Box::new(Expr::Local(index)),
                    inc: false,
                    prefix: false,
                })),
                Op::Dup => {
                    self.serial += 1;
                    let top = stack.last_mut().ok_or("dup on an empty stack")?;
                    // A stored value used twice must be stored once: the store becomes a statement.
                    if let Expr::Assign(target, _) = &top.e
                        && matches!(target.stripped(), Expr::Local(_))
                    {
                        let local = target.stripped().clone();
                        stmts.push(Stmt::Expr(std::mem::replace(&mut top.e, local)));
                    }
                    if top.dup == 0 {
                        top.dup = self.serial;
                    }
                    let copy = top.clone();
                    stack.push(copy);
                }
                Op::Swap => {
                    let n = stack.len();
                    if n < 2 {
                        return Err("swap needs two values".into());
                    }
                    stack.swap(n - 1, n - 2);
                }
                Op::Pop => {
                    let v = pop(&mut stack)?;
                    // A discarded property read can still run a getter, so it stays a statement.
                    let inert = matches!(
                        v.e.stripped(),
                        Expr::Lit(_) | Expr::Local(_) | Expr::This | Expr::In(_) | Expr::Activation | Expr::CatchScope(_)
                            | Expr::Function(_) | Expr::Global | Expr::Scope(_)
                    );
                    if !inert && !stack.iter().any(|s| s.dup != 0 && s.dup == v.dup) {
                        stmts.push(Stmt::Expr(v.e));
                    }
                }
                Op::GetLex { index } => {
                    let attribute = self.abc.key_parts(index).attribute;
                    let name = self.abc.simple(index);
                    push!(Expr::Name(if attribute { format!("@{name}") } else { name }))
                }
                Op::FindPropStrict { index } | Op::FindProperty { index } | Op::FindDef { index } => {
                    let key = self.key(index, &mut stack)?;
                    push!(match key {
                        Key::Name(n) => Expr::Scope(n),
                        _ => Expr::Global,
                    })
                }
                Op::GetProperty { index } => {
                    let key = self.key(index, &mut stack)?;
                    let obj = pop(&mut stack)?.e;
                    push!(prop(obj, key));
                }
                Op::SetProperty { index } | Op::InitProperty { index } => {
                    let v = pop(&mut stack)?;
                    let key = self.key(index, &mut stack)?;
                    let obj = pop(&mut stack)?;
                    // `new <T>[a, b]`: a sized vector filled in place through copies of itself.
                    if obj.dup != 0
                        && let Key::Index(slot) = &key
                        && let Expr::Lit(Lit::Int(slot)) = slot.stripped()
                        && let Some(base) = stack.iter_mut().rev().find(|s| s.dup == obj.dup)
                        && let Some(items) = vector_items(&mut base.e, *slot)
                    {
                        items.push(v.e);
                        continue;
                    }
                    let obj = obj.e;
                    if obj == Expr::Raw("§named".into())
                        && let (Expr::Function(marker), Key::Name(name)) = (&v.e, &key)
                        && let Some(n) = marker.trim_matches('§').strip_prefix("closure").and_then(|n| n.parse::<usize>().ok())
                    {
                        if let Some(closure) = self.closures.borrow_mut().get_mut(n) {
                            closure.1 = Some(name.clone());
                        }
                        if let Some(twin) = stack.iter_mut().rev().find(|s| v.dup != 0 && s.dup == v.dup) {
                            twin.dup = 0;
                        }
                        continue;
                    }
                    let target = prop(obj, key);
                    self.assign(target, v, &mut stack, &mut stmts, &mut temps);
                }
                Op::DeleteProperty { index } => {
                    let key = self.key(index, &mut stack)?;
                    let obj = pop(&mut stack)?.e;
                    push!(Expr::Delete(Box::new(prop(obj, key))));
                }
                Op::GetDescendants { index } => {
                    let key = self.key(index, &mut stack)?;
                    let obj = pop(&mut stack)?.e;
                    let key = match key {
                        Key::Name(n) | Key::Attr(n) => Key::Descendants(n),
                        other => other,
                    };
                    push!(Expr::Prop(Box::new(obj), key));
                }
                Op::GetSuper { index } => {
                    let key = self.key(index, &mut stack)?;
                    pop(&mut stack)?;
                    push!(Expr::Super(key));
                }
                Op::SetSuper { index } => {
                    let v = pop(&mut stack)?.e;
                    let key = self.key(index, &mut stack)?;
                    pop(&mut stack)?;
                    stmts.push(Stmt::Expr(Expr::Assign(Box::new(Expr::Super(key)), Box::new(v))));
                }
                Op::CallProperty { index, num_args } | Op::CallPropLex { index, num_args } | Op::CallPropVoid { index, num_args } => {
                    let args = pop_n(&mut stack, num_args)?;
                    let key = self.key(index, &mut stack)?;
                    let obj = pop(&mut stack)?.e;
                    let call = Expr::Call(Box::new(prop(obj, key)), args);
                    if matches!(op, Op::CallPropVoid { .. }) {
                        stmts.push(Stmt::Expr(call));
                    } else {
                        push!(call);
                    }
                }
                Op::CallSuper { index, num_args } | Op::CallSuperVoid { index, num_args } => {
                    let args = pop_n(&mut stack, num_args)?;
                    let key = self.key(index, &mut stack)?;
                    pop(&mut stack)?;
                    let call = Expr::Call(Box::new(Expr::Super(key)), args);
                    if matches!(op, Op::CallSuperVoid { .. }) {
                        stmts.push(Stmt::Expr(call));
                    } else {
                        push!(call);
                    }
                }
                Op::Call { num_args } => {
                    let args = pop_n(&mut stack, num_args)?;
                    pop(&mut stack)?;
                    let f = pop(&mut stack)?.e;
                    push!(Expr::Call(Box::new(f), args));
                }
                Op::CallMethod { index, num_args } => {
                    let args = pop_n(&mut stack, num_args)?;
                    let obj = pop(&mut stack)?.e;
                    push!(Expr::Call(Box::new(Expr::Prop(Box::new(obj), Key::Name(format!("method{index}")))), args));
                }
                Op::CallStatic { index, num_args } => {
                    let args = pop_n(&mut stack, num_args)?;
                    pop(&mut stack)?;
                    push!(Expr::Call(Box::new(Expr::Raw(format!("method#{}", index.0))), args));
                }
                Op::Construct { num_args } => {
                    let args = pop_n(&mut stack, num_args)?;
                    let ctor = pop(&mut stack)?.e;
                    push!(Expr::New(Box::new(ctor), args));
                }
                Op::ConstructProp { index, num_args } => {
                    let args = pop_n(&mut stack, num_args)?;
                    let key = self.key(index, &mut stack)?;
                    let obj = pop(&mut stack)?.e;
                    push!(Expr::New(Box::new(prop(obj, key)), args));
                }
                Op::ConstructSuper { num_args } => {
                    let args = pop_n(&mut stack, num_args)?;
                    pop(&mut stack)?;
                    stmts.push(Stmt::Expr(Expr::Call(Box::new(Expr::Raw("super".into())), args)));
                }
                Op::NewFunction { index } => {
                    let mut closures = self.closures.borrow_mut();
                    closures.push((index, None));
                    push!(Expr::Function(format!("§closure{}§", closures.len() - 1)));
                }
                Op::NewClass { index } => {
                    pop(&mut stack)?;
                    let name = self.abc.abc.instances.get(index.0 as usize).map_or("?".into(), |i| self.abc.simple(i.name));
                    push!(Expr::Name(name));
                }
                Op::NewObject { num_args } => {
                    let flat = pop_n(&mut stack, num_args * 2)?;
                    let pairs = flat.chunks(2).map(|p| (p[0].clone(), p[1].clone())).collect();
                    push!(Expr::Object(pairs));
                }
                Op::NewArray { num_args } => {
                    let items = pop_n(&mut stack, num_args)?;
                    push!(Expr::Array(items));
                }
                Op::NewActivation => push!(Expr::Activation),
                Op::NewCatch { index } => push!(Expr::CatchScope(index.0)),
                Op::GetSlot { index } => {
                    let obj = pop(&mut stack)?.e;
                    push!(self.slot(obj, index));
                }
                Op::SetSlot { index } => {
                    let v = pop(&mut stack)?;
                    let obj = pop(&mut stack)?.e;
                    match obj {
                        Expr::CatchScope(_) => {}
                        Expr::Activation => {
                            let name = self.slot_name(index).unwrap_or_else(|| format!("slot{index}"));
                            // The prologue copies arguments into the activation; keep their names instead.
                            if let Expr::Local(r) = v.e.stripped()
                                && *r >= 1
                                && *r <= self.params
                            {
                                self.names.borrow_mut().insert(*r, name);
                            } else {
                                self.assign(Expr::Name(name), v, &mut stack, &mut stmts, &mut temps);
                            }
                        }
                        other => {
                            let target = self.slot(other, index);
                            self.assign(target, v, &mut stack, &mut stmts, &mut temps);
                        }
                    }
                }
                Op::GetGlobalSlot { index } => push!(Expr::Name(self.abc.global_slot(index))),
                Op::SetGlobalSlot { index } => {
                    let v = pop(&mut stack)?;
                    self.assign(Expr::Name(self.abc.global_slot(index)), v, &mut stack, &mut stmts, &mut temps);
                }
                Op::GetGlobalScope => push!(Expr::Global),
                Op::GetScopeObject { index } => push!(scopes.get(index as usize).cloned().unwrap_or(Expr::Global)),
                Op::GetOuterScope { index } => push!(Expr::Raw(format!("§outer{index}"))),
                Op::PushScope => {
                    let v = pop(&mut stack)?.e;
                    scopes.push(v);
                }
                Op::PushWith => {
                    let v = pop(&mut stack)?.e;
                    // An empty object as scope holds the name of a named function expression.
                    if v == Expr::Object(vec![]) {
                        scopes.push(Expr::Raw("§named".into()));
                    } else {
                        stmts.push(Stmt::Comment(format!("with scope: {v:?}")));
                        scopes.push(v);
                    }
                }
                Op::PopScope => {
                    scopes.pop();
                }
                Op::HasNext => {
                    let index = pop(&mut stack)?.e;
                    let obj = pop(&mut stack)?.e;
                    push!(Expr::Call(Box::new(Expr::Raw("hasnext".into())), vec![obj, index]));
                }
                Op::HasNext2 { object_register, index_register } => push!(Expr::HasNext(object_register, index_register)),
                Op::NextName => {
                    let index = pop(&mut stack)?.e;
                    let obj = pop(&mut stack)?.e;
                    push!(Expr::NextName(Box::new(obj), Box::new(index)));
                }
                Op::NextValue => {
                    let index = pop(&mut stack)?.e;
                    let obj = pop(&mut stack)?.e;
                    push!(Expr::NextValue(Box::new(obj), Box::new(index)));
                }
                Op::Coerce { index } => coerce(&mut stack, self.abc.type_name(index))?,
                Op::CoerceA => coerce(&mut stack, "*".into())?,
                Op::CoerceS | Op::ConvertS => coerce(&mut stack, "String".into())?,
                Op::CoerceI | Op::ConvertI => coerce(&mut stack, "int".into())?,
                Op::CoerceU | Op::ConvertU => coerce(&mut stack, "uint".into())?,
                Op::CoerceD | Op::ConvertD => coerce(&mut stack, "Number".into())?,
                Op::CoerceB | Op::ConvertB => coerce(&mut stack, "Boolean".into())?,
                Op::CoerceO | Op::ConvertO => coerce(&mut stack, "Object".into())?,
                Op::AsType { type_name } => {
                    let v = pop(&mut stack)?.e;
                    push!(Expr::bin(BinOp::As, v, Expr::Name(self.abc.type_name(type_name))));
                }
                Op::IsType { index } => {
                    let v = pop(&mut stack)?.e;
                    push!(Expr::bin(BinOp::Is, v, Expr::Name(self.abc.type_name(index))));
                }
                Op::AsTypeLate => binary!(BinOp::As),
                Op::IsTypeLate => binary!(BinOp::Is),
                Op::InstanceOf => binary!(BinOp::InstanceOf),
                Op::In => binary!(BinOp::In),
                Op::TypeOf => {
                    let v = pop(&mut stack)?.e;
                    push!(Expr::Unary(UnOp::TypeOf, Box::new(v)));
                }
                Op::Not => {
                    let v = pop(&mut stack)?.e;
                    push!(negate(v));
                }
                Op::Negate | Op::NegateI => {
                    let v = pop(&mut stack)?.e;
                    push!(match v.stripped() {
                        Expr::Lit(Lit::Int(n)) => Expr::int(-n),
                        Expr::Lit(Lit::Num(n)) => Expr::Lit(Lit::Num(-n)),
                        _ => Expr::Unary(UnOp::Neg, Box::new(v)),
                    });
                }
                Op::BitNot => {
                    let v = pop(&mut stack)?.e;
                    push!(Expr::Unary(UnOp::BitNot, Box::new(v)));
                }
                Op::Increment | Op::IncrementI | Op::Decrement | Op::DecrementI => {
                    let v = pop(&mut stack)?;
                    let op = if matches!(op, Op::Increment | Op::IncrementI) { BinOp::Add } else { BinOp::Sub };
                    stack.push(Val { e: Expr::bin(op, v.e, Expr::int(1)), dup: 0, inc_of: v.dup, temp: 0 });
                }
                Op::Add | Op::AddI => binary!(BinOp::Add),
                Op::Subtract | Op::SubtractI => binary!(BinOp::Sub),
                Op::Multiply | Op::MultiplyI => binary!(BinOp::Mul),
                Op::Divide => binary!(BinOp::Div),
                Op::Modulo => binary!(BinOp::Mod),
                Op::LShift => binary!(BinOp::Shl),
                Op::RShift => binary!(BinOp::Shr),
                Op::URShift => binary!(BinOp::Ushr),
                Op::BitAnd => binary!(BinOp::BitAnd),
                Op::BitOr => binary!(BinOp::BitOr),
                Op::BitXor => binary!(BinOp::BitXor),
                Op::Equals => binary!(BinOp::Eq),
                Op::StrictEquals => binary!(BinOp::StrictEq),
                Op::LessThan => binary!(BinOp::Lt),
                Op::LessEquals => binary!(BinOp::Le),
                Op::GreaterThan => binary!(BinOp::Gt),
                Op::GreaterEquals => binary!(BinOp::Ge),
                Op::ApplyType { num_types } => {
                    let args = pop_n(&mut stack, num_types)?;
                    let base = pop(&mut stack)?.e;
                    push!(Expr::TypeArgs(Box::new(base), args));
                }
                Op::CheckFilter => {}
                Op::EscXElem | Op::EscXAttr => {
                    let v = pop(&mut stack)?.e;
                    push!(Expr::Call(Box::new(Expr::Raw("escapeXML".into())), vec![v]));
                }
                Op::Dxns { index } => {
                    stmts.push(Stmt::Comment(format!("default xml namespace = {}", self.abc.string(index))));
                }
                Op::DxnsLate => {
                    let v = pop(&mut stack)?.e;
                    stmts.push(Stmt::Expr(Expr::Assign(Box::new(Expr::Raw("default xml namespace".into())), Box::new(v))));
                }
                Op::Li8 | Op::Li16 | Op::Li32 | Op::Lf32 | Op::Lf64 | Op::Sxi1 | Op::Sxi8 | Op::Sxi16 => {
                    let v = pop(&mut stack)?.e;
                    let name = format!("{op:?}").to_lowercase();
                    push!(Expr::Call(Box::new(Expr::Raw(name)), vec![v]));
                }
                Op::Si8 | Op::Si16 | Op::Si32 | Op::Sf32 | Op::Sf64 => {
                    let address = pop(&mut stack)?.e;
                    let v = pop(&mut stack)?.e;
                    let name = format!("{op:?}").to_lowercase();
                    stmts.push(Stmt::Expr(Expr::Call(Box::new(Expr::Raw(name)), vec![v, address])));
                }
                Op::ReturnVoid => term = Some(Term::Return(None)),
                Op::ReturnValue => term = Some(Term::Return(Some(pop(&mut stack)?.e))),
                Op::Throw => term = Some(Term::Throw(pop(&mut stack)?.e)),
                Op::Jump { offset } => term = Some(Term::Goto(next_block(self, branch_target(next, offset))?)),
                Op::IfTrue { offset } => {
                    let c = pop(&mut stack)?.e;
                    cond!(offset, c)
                }
                Op::IfFalse { offset } => {
                    let c = pop(&mut stack)?.e;
                    cond!(offset, negate(c))
                }
                Op::IfEq { offset } => compare!(offset, BinOp::Eq, false),
                Op::IfNe { offset } => compare!(offset, BinOp::Ne, false),
                Op::IfLt { offset } => compare!(offset, BinOp::Lt, false),
                Op::IfLe { offset } => compare!(offset, BinOp::Le, false),
                Op::IfGt { offset } => compare!(offset, BinOp::Gt, false),
                Op::IfGe { offset } => compare!(offset, BinOp::Ge, false),
                Op::IfNlt { offset } => compare!(offset, BinOp::Lt, true),
                Op::IfNle { offset } => compare!(offset, BinOp::Le, true),
                Op::IfNgt { offset } => compare!(offset, BinOp::Gt, true),
                Op::IfNge { offset } => compare!(offset, BinOp::Ge, true),
                Op::IfStrictEq { offset } => compare!(offset, BinOp::StrictEq, false),
                Op::IfStrictNe { offset } => compare!(offset, BinOp::StrictNe, false),
                Op::LookupSwitch(table) => {
                    let index = pop(&mut stack)?.e;
                    let at = self.ops[i].0 as i64;
                    let default = next_block(self, at + table.default_offset as i64)?;
                    let cases = table
                        .case_offsets
                        .iter()
                        .map(|c| next_block(self, at + *c as i64))
                        .collect::<Result<Vec<_>>>()?;
                    term = Some(Term::Switch(index, cases, default));
                }
            }
        }
        let block = &mut self.blocks[b];
        block.stmts = stmts.into_iter().enumerate().filter(|(i, _)| !temps.removed.contains(i)).map(|(_, s)| s).collect();
        block.exit = stack.into_iter().map(|v| v.e).collect();
        block.term = match term {
            Some(t) => t,
            None if end < self.ops.len() => Term::Goto(self.block_of[end]),
            None => Term::End,
        };
        Ok(scopes)
    }

    /// A slot whose object was still a stack placeholder when it was read.
    fn resolve_slot(&self, e: &Expr) -> Option<Expr> {
        let Expr::Prop(obj, Key::Name(name)) = e else { return None };
        let slot = name.strip_prefix("§slot")?.parse::<u32>().ok()?;
        let resolved = self.slot(obj.stripped().clone(), slot);
        (resolved != *e).then_some(resolved)
    }

    fn slot(&self, obj: Expr, index: u32) -> Expr {
        match obj {
            Expr::Activation => Expr::Name(self.slot_name(index).unwrap_or_else(|| format!("slot{index}"))),
            Expr::CatchScope(n) => Expr::Name(self.catch_name(n as usize).0),
            Expr::Global => Expr::Name(self.abc.global_slot(index)),
            other => match self.abc.slot_of(&other, index) {
                Some(name) => prop(other, Key::Name(name)),
                None => Expr::Prop(Box::new(other), Key::Name(format!("§slot{index}"))),
            },
        }
    }

    fn set_local(&mut self, index: u32, v: Val, stack: &mut [Val], stmts: &mut Vec<Stmt>, temps: &mut Temps) {
        if matches!(v.e, Expr::Activation | Expr::CatchScope(_)) {
            return;
        }
        self.stored.entry(index).or_default().push(super::expr::value_type(&v.e));
        // One value kept for several later uses (`dup; dup; setlocal`) is stored once and read back.
        if v.dup != 0 {
            let copies = stack.iter().filter(|s| s.dup == v.dup).count();
            let twin_on_top = stack.last().is_some_and(|top| top.dup == v.dup);
            if copies > 1 || (copies == 1 && !twin_on_top) {
                if let Expr::Coerce(kind, _) = &v.e {
                    self.types.entry(index).or_insert_with(|| kind.clone());
                }
                stmts.push(Stmt::Expr(Expr::Assign(Box::new(Expr::Local(index)), Box::new(v.e))));
                for copy in stack.iter_mut().filter(|s| s.dup == v.dup) {
                    *copy = Val::new(Expr::Local(index));
                }
                return;
            }
        }
        if self.temps.contains(&index) {
            temps.held.remove(&index);
            let quiet = !v.e.has_effects();
            match stack.last_mut().filter(|top| v.dup != 0 && top.dup == v.dup) {
                Some(twin) => {
                    twin.dup = 0;
                    twin.temp = index;
                    if !quiet {
                        twin.e = Expr::Assign(Box::new(Expr::Local(index)), Box::new(v.e));
                        return;
                    }
                }
                None if !quiet => {
                    stmts.push(Stmt::Expr(Expr::Assign(Box::new(Expr::Local(index)), Box::new(v.e))));
                    return;
                }
                None => {}
            }
            let at = stmts.len();
            stmts.push(Stmt::Expr(Expr::Assign(Box::new(Expr::Local(index)), Box::new(v.e.clone()))));
            temps.plain.insert(at);
            temps.held.insert(index, Held { at, value: Val { dup: 0, temp: 0, ..v }, target: None });
            return;
        }
        if let Expr::Coerce(kind, _) = &v.e {
            self.types.entry(index).or_insert_with(|| kind.clone());
        }
        if self.chain(&Expr::Local(index), &v, stmts, temps) {
            return;
        }
        if let Some(top) = stack.last_mut() {
            if v.dup != 0 && top.dup == v.dup {
                top.e = Expr::Assign(Box::new(Expr::Local(index)), Box::new(v.e));
                top.dup = 0;
                return;
            }
            if v.inc_of != 0 && top.dup == v.inc_of && *top.e.stripped() == Expr::Local(index) {
                let inc = matches!(v.e.stripped(), Expr::Binary(BinOp::Add, ..));
                top.e = Expr::Update { target: Box::new(Expr::Local(index)), inc, prefix: false };
                top.dup = 0;
                return;
            }
        }
        stmts.push(Stmt::Expr(Expr::Assign(Box::new(Expr::Local(index)), Box::new(v.e))));
    }

    fn assign(&mut self, target: Expr, v: Val, stack: &mut [Val], stmts: &mut Vec<Stmt>, temps: &mut Temps) {
        if let Some(top) = stack.last_mut()
            && v.dup != 0
            && top.dup == v.dup
        {
            top.e = Expr::Assign(Box::new(target), Box::new(v.e));
            top.dup = 0;
            return;
        }
        // `obj.p++` used as a value: the old value stays on the stack.
        if v.inc_of != 0
            && let Some(old) = stack.iter_mut().rev().find(|s| s.dup == v.inc_of)
            && old.e.stripped() == target.stripped()
        {
            let inc = matches!(v.e.stripped(), Expr::Binary(BinOp::Add, ..));
            old.e = Expr::Update { target: Box::new(target), inc, prefix: false };
            old.dup = 0;
            return;
        }
        if self.chain(&target, &v, stmts, temps) {
            return;
        }
        stmts.push(Stmt::Expr(Expr::Assign(Box::new(target), Box::new(v.e))));
    }

    /// `a = b = value` through a temporary: the temporary's store becomes this assignment,
    /// which a later read of the temporary takes as its value.
    fn chain(&self, target: &Expr, v: &Val, stmts: &mut Vec<Stmt>, temps: &mut Temps) -> bool {
        let t = v.temp;
        if t == 0 {
            return false;
        }
        if let Some(h) = temps.held.get_mut(&t)
            && h.target.is_none()
            && !temps.removed.contains(&h.at)
            && (h.at + 1..stmts.len()).all(|j| temps.removed.contains(&j) || temps.plain.contains(&j))
        {
            let chained = Expr::Assign(Box::new(Expr::Local(t)), Box::new(h.value.e.clone()));
            stmts[h.at] = Stmt::Expr(Expr::Assign(Box::new(target.clone()), Box::new(chained)));
            temps.plain.remove(&h.at);
            h.target = Some(target.clone());
            return true;
        }
        if let Expr::Assign(local, inner) = &v.e
            && **local == Expr::Local(t)
        {
            temps.held.insert(t, Held { at: stmts.len(), value: Val::new((**inner).clone()), target: Some(target.clone()) });
            stmts.push(Stmt::Expr(Expr::Assign(Box::new(target.clone()), Box::new(v.e.clone()))));
            return true;
        }
        false
    }

    // ---------------------------------------------------------------- reduction

    fn preds(&self) -> Vec<Vec<usize>> {
        let mut preds = vec![Vec::new(); self.blocks.len()];
        for (b, block) in self.blocks.iter().enumerate() {
            if block.alive {
                for s in block.term.succs() {
                    if !preds[s].contains(&b) {
                        preds[s].push(b);
                    }
                }
            }
        }
        preds
    }

    fn try_key(&self, b: usize) -> Vec<usize> {
        let offset = self.blocks[b].offset as u32;
        self.body
            .exceptions
            .iter()
            .enumerate()
            .filter(|(_, e)| offset >= e.from_offset && offset < e.to_offset)
            .map(|(n, _)| n)
            .collect()
    }

    fn is_try_start(&self, b: usize) -> bool {
        let offset = self.blocks[b].offset as u32;
        self.body.exceptions.iter().any(|e| e.from_offset == offset)
    }

    fn reduce(&mut self) {
        loop {
            let preds = self.preds();
            let mut changed = false;
            for b in 0..self.blocks.len() {
                if !self.blocks[b].alive {
                    continue;
                }
                if self.try_value(b, &preds)
                    || self.try_ternary(b, &preds)
                    || self.try_compound(b, &preds)
                    || self.try_merge(b, &preds)
                    || self.try_thread(b, &preds)
                {
                    changed = true;
                    break;
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn value_block(&self, y: usize, preds: &[Vec<usize>], from: usize) -> Option<usize> {
        let block = &self.blocks[y];
        match block.term {
            Term::Goto(j) if block.alive && block.stmts.is_empty() && preds[y] == [from] && block.handler.is_none() => Some(j),
            _ => None,
        }
    }

    /// `a && b`, `a || b` and their general form: a branch that leaves a value for the block it joins.
    fn try_value(&mut self, x: usize, preds: &[Vec<usize>]) -> bool {
        let Term::Cond(c, t, f) = self.blocks[x].term.clone() else { return false };
        let d = self.blocks[x].exit.len();
        if d == 0 || t == f {
            return false;
        }
        for (j, y, when) in [(t, f, c.clone()), (f, t, negate(c.clone()))] {
            if self.blocks[j].depth != d || self.blocks[y].depth != d || self.value_block(y, preds, x) != Some(j) {
                continue;
            }
            let lower = &self.blocks[x].exit;
            let ys: Vec<Expr> = self.blocks[y].exit.iter().map(|e| e.substitute(lower)).collect();
            if ys.len() != d || ys[..d - 1] != lower[..d - 1] {
                continue;
            }
            let value = ternary(when, lower[d - 1].clone(), ys[d - 1].clone());
            self.blocks[x].exit[d - 1] = value;
            self.blocks[x].term = Term::Goto(j);
            self.blocks[y].alive = false;
            return true;
        }
        false
    }

    fn try_ternary(&mut self, x: usize, preds: &[Vec<usize>]) -> bool {
        let Term::Cond(c, t, f) = self.blocks[x].term.clone() else { return false };
        if t == f {
            return false;
        }
        let (Some(jt), Some(jf)) = (self.value_block(t, preds, x), self.value_block(f, preds, x)) else { return false };
        let d = self.blocks[x].exit.len();
        if jt != jf || self.blocks[jt].depth != d + 1 || self.blocks[t].depth != d || self.blocks[f].depth != d {
            return false;
        }
        let lower = self.blocks[x].exit.clone();
        let vt: Vec<Expr> = self.blocks[t].exit.iter().map(|e| e.substitute(&lower)).collect();
        let vf: Vec<Expr> = self.blocks[f].exit.iter().map(|e| e.substitute(&lower)).collect();
        if vt.len() != d + 1 || vf.len() != d + 1 || vt[..d] != lower[..] || vf[..d] != lower[..] {
            return false;
        }
        // The fallthrough branch is laid out first, so it reads as the `then` value.
        let value = if self.blocks[f].offset <= self.blocks[t].offset {
            ternary(negate(c), vf[d].clone(), vt[d].clone())
        } else {
            ternary(c, vt[d].clone(), vf[d].clone())
        };
        self.blocks[x].exit.push(value);
        self.blocks[x].term = Term::Goto(jt);
        self.blocks[t].alive = false;
        self.blocks[f].alive = false;
        true
    }

    /// Two condition blocks sharing a target become one: `if(a && b)`, `while(a || b)`.
    fn try_compound(&mut self, x: usize, preds: &[Vec<usize>]) -> bool {
        let Term::Cond(cx, xt, xf) = self.blocks[x].term.clone() else { return false };
        if !self.blocks[x].exit.is_empty() || xt == xf {
            return false;
        }
        for (y, other) in [(xt, xf), (xf, xt)] {
            let block = &self.blocks[y];
            let Term::Cond(cy, yt, yf) = block.term.clone() else { continue };
            // A test that stores a local reads better as its own statement in a nested if.
            if y == x || !block.alive || !block.stmts.is_empty() || !block.exit.is_empty() || block.depth != 0
                || preds[y] != [x] || block.handler.is_some() || self.try_key(y) != self.try_key(x)
                || cy.any(&|e| matches!(e, Expr::Assign(..)))
            {
                continue;
            }
            let to_y = if y == xt { cx.clone() } else { invert(cx.clone()) };
            let term = if yt == other {
                Term::Cond(Expr::bin(BinOp::Or, invert(to_y), cy), other, yf)
            } else if yf == other {
                Term::Cond(Expr::bin(BinOp::And, to_y, cy), yt, other)
            } else {
                continue;
            };
            if let Term::Cond(c, ..) = &term {
                self.compounds.push(c.clone());
            }
            self.blocks[x].term = term;
            self.blocks[y].alive = false;
            return true;
        }
        false
    }

    fn try_merge(&mut self, p: usize, preds: &[Vec<usize>]) -> bool {
        let Term::Goto(j) = self.blocks[p].term else { return false };
        if j == p || j == 0 || preds[j] != [p] || self.blocks[j].handler.is_some() || !self.blocks[j].alive
            || self.try_key(j) != self.try_key(p) || self.is_try_start(j)
        {
            return false;
        }
        let values = self.blocks[p].exit.clone();
        if self.blocks[j].depth != values.len() {
            return false;
        }
        let stmts: Vec<Stmt> = self.blocks[j].stmts.iter().map(|s| substitute_stmt(s, &values)).collect();
        let exit: Vec<Expr> = self.blocks[j].exit.iter().map(|e| e.substitute(&values)).collect();
        let term = self.blocks[j].term.substitute(&values);
        let end = self.blocks[j].end;
        let block = &mut self.blocks[p];
        block.stmts.extend(stmts);
        block.exit = exit;
        block.term = term;
        block.end = end;
        self.blocks[j].alive = false;
        true
    }

    /// An empty block that only jumps on is skipped by its predecessors.
    fn try_thread(&mut self, b: usize, preds: &[Vec<usize>]) -> bool {
        let block = &self.blocks[b];
        let Term::Goto(k) = block.term else { return false };
        let passes = block.exit.iter().enumerate().all(|(i, e)| *e == Expr::In(i)) && block.exit.len() == block.depth;
        if k == b || b == 0 || !block.stmts.is_empty() || !passes || block.handler.is_some() || preds[b].is_empty()
            || self.is_try_start(b)
        {
            return false;
        }
        for &p in &preds[b] {
            self.blocks[p].term.retarget(b, k);
            if let Term::Cond(c, t, f) = &self.blocks[p].term
                && t == f
            {
                let (c, t) = (c.clone(), *t);
                // `if(c) {}`: the condition still runs.
                if !matches!(c.stripped(), Expr::Lit(_) | Expr::Local(_) | Expr::This | Expr::In(_)) {
                    self.blocks[p].stmts.push(Stmt::If(c, vec![], vec![]));
                }
                self.blocks[p].term = Term::Goto(t);
            }
        }
        self.blocks[b].alive = false;
        true
    }

    // ---------------------------------------------------------------- structuring

    fn succs(&self, b: usize) -> Vec<usize> {
        self.blocks[b].term.succs().into_iter().filter(|s| self.blocks[*s].alive).collect()
    }

    fn roots(&self) -> Vec<usize> {
        let mut roots = vec![0];
        roots.extend((0..self.blocks.len()).filter(|&b| self.blocks[b].alive && self.blocks[b].handler.is_some()));
        roots
    }

    fn find_loops(&mut self) {
        // Depth-first search for back edges whose target dominates their source.
        let n = self.blocks.len();
        let roots = self.roots();
        let dom = dominators(n, &roots, &|b| self.succs(b));
        let mut state = vec![0u8; n];
        let mut back: Vec<(usize, usize)> = Vec::new();
        for root in roots {
            if state[root] != 0 {
                continue;
            }
            let mut stack = vec![(root, 0usize)];
            state[root] = 1;
            while let Some((b, i)) = stack.pop() {
                let succs = self.succs(b);
                if i < succs.len() {
                    stack.push((b, i + 1));
                    let s = succs[i];
                    match state[s] {
                        0 => {
                            state[s] = 1;
                            stack.push((s, 0));
                        }
                        1 => back.push((b, s)),
                        _ => {}
                    }
                } else {
                    state[b] = 2;
                }
            }
        }
        let preds = self.preds();
        for (tail, head) in back {
            if !dominates(&dom, head, tail) {
                continue;
            }
            let body = self.loops.entry(head).or_insert_with(|| BTreeSet::from([head]));
            let mut work = vec![tail];
            while let Some(b) = work.pop() {
                if body.insert(b) {
                    work.extend(preds[b].iter().copied());
                }
            }
        }
    }

    pub fn structure(&mut self) -> Result<Vec<Stmt>> {
        self.build_blocks()?;
        self.find_temps();
        self.simulate_all()?;
        self.reduce();
        self.find_loops();
        self.done = vec![false; self.blocks.len()];
        let mut out = Vec::new();
        let mut frames = Vec::new();
        self.region(0, None, &mut frames, &mut out);
        map_stmts(&mut out, &|e| self.resolve_slot(e));
        hoist_assignments(&mut out);
        Ok(out)
    }

    fn jump_stmt(&self, b: usize, frames: &mut [Frame]) -> Option<Stmt> {
        let last = frames.len().checked_sub(1)?;
        for (i, frame) in frames.iter_mut().enumerate().rev() {
            if frame.brk == Some(b) {
                return Some(if i == last {
                    Stmt::Break(None)
                } else {
                    frame.used = true;
                    Stmt::Break(Some(frame.label.clone()))
                });
            }
            if frame.cont == Some(b) {
                // `continue` applies to the innermost loop; switches have no continue target.
                let innermost_loop = frames_innermost_loop(i, last);
                return Some(if innermost_loop {
                    Stmt::Continue(None)
                } else {
                    frame.used = true;
                    Stmt::Continue(Some(frame.label.clone()))
                });
            }
        }
        None
    }

    fn region(&mut self, start: usize, stop: Option<usize>, frames: &mut Vec<Frame>, out: &mut Vec<Stmt>) {
        let mut cur = start;
        loop {
            if Some(cur) == stop {
                return;
            }
            if let Some(s) = self.jump_stmt_checked(cur, frames) {
                out.push(s);
                return;
            }
            if self.done[cur] {
                out.push(Stmt::Comment(format!("goto byte {}", self.blocks[cur].offset)));
                return;
            }
            if let Some(next) = self.try_region(cur, stop, frames, out) {
                match next {
                    Some(n) => {
                        cur = n;
                        continue;
                    }
                    None => return,
                }
            }
            if self.loops.contains_key(&cur) && !self.entered.contains(&cur) {
                match self.emit_loop(cur, frames, out) {
                    Some(n) => {
                        cur = n;
                        continue;
                    }
                    None => return,
                }
            }
            self.done[cur] = true;
            out.extend(self.blocks[cur].stmts.clone());
            match self.blocks[cur].term.clone() {
                Term::Return(e) => {
                    out.push(Stmt::Return(e));
                    return;
                }
                Term::Throw(e) => {
                    out.push(Stmt::Throw(e));
                    return;
                }
                Term::End => return,
                Term::Goto(t) => cur = t,
                Term::Cond(c, t, f) => match self.emit_if(cur, c, t, f, stop, frames, out) {
                    Some(n) => cur = n,
                    None => return,
                },
                Term::Switch(index, cases, default) => match self.emit_switch(cur, index, cases, default, stop, frames, out) {
                    Some(n) => cur = n,
                    None => return,
                },
            }
        }
    }

    fn jump_stmt_checked(&self, b: usize, frames: &mut [Frame]) -> Option<Stmt> {
        if frames.is_empty() {
            return None;
        }
        self.jump_stmt(b, frames)
    }

    /// The innermost loop around `b`, used to bound merge-point searches.
    fn region_nodes(&self, frames: &[Frame]) -> Option<&BTreeSet<usize>> {
        frames
            .iter()
            .rev()
            .find_map(|f| f.cont.and_then(|c| self.loops.iter().find(|(h, body)| **h == c || body.contains(&c)).map(|(_, body)| body)))
    }

    /// The block both branches of `b` reach next, if any, before leaving the current region.
    fn merge_point(&self, b: usize, stop: Option<usize>, frames: &[Frame]) -> Option<usize> {
        let region = self.region_nodes(frames);
        let exits: HashSet<usize> = frames.iter().flat_map(|f| [f.brk, f.cont]).flatten().collect();
        // Branches that both end in the same `break` meet there: `if {} else {} break;`.
        let brk = frames.last().and_then(|f| f.brk);
        let inside = |n: usize| -> bool {
            self.blocks[n].alive
                && !exits.contains(&n)
                && region.is_none_or(|r| r.contains(&n))
                && frames.iter().all(|f| f.cont != Some(n))
        };
        // Post-dominators over: region blocks, `stop` as a sink, and one virtual exit.
        let n = self.blocks.len();
        let exit = n;
        let succ = |v: usize| -> Vec<usize> {
            if v == exit {
                return vec![];
            }
            if Some(v) == stop || Some(v) == brk {
                return vec![exit];
            }
            let mut out = Vec::new();
            let all = self.succs(v);
            if all.is_empty() {
                out.push(exit);
            }
            for s in all {
                if Some(s) == stop || Some(s) == brk || inside(s) {
                    out.push(s);
                } else {
                    out.push(exit);
                }
            }
            out
        };
        // Collect the nodes reachable from b, then compute post-dominators on the reversed graph.
        let mut nodes = vec![b];
        let mut seen = HashSet::from([b]);
        let mut i = 0;
        while i < nodes.len() {
            for s in succ(nodes[i]) {
                if seen.insert(s) {
                    nodes.push(s);
                }
            }
            i += 1;
        }
        if !seen.contains(&exit) {
            return None;
        }
        let index: HashMap<usize, usize> = nodes.iter().enumerate().map(|(i, v)| (*v, i)).collect();
        let m = nodes.len();
        let mut rev = vec![Vec::new(); m];
        for (i, v) in nodes.iter().enumerate() {
            for s in succ(*v) {
                rev[index[&s]].push(i);
            }
        }
        let root = index[&exit];
        let pdom = dominators(m, &[root], &|v| rev[v].clone());
        let mut p = pdom[index[&b]];
        // Skip `b` itself.
        while let Some(v) = p {
            if v == root {
                return None;
            }
            if v != index[&b] {
                return Some(nodes[v]);
            }
            p = pdom[v];
        }
        None
    }

    /// With early returns no block post-dominates both branches; the block both still reach
    /// soonest is where they join.
    fn common_join(&self, b: usize, t: usize, f: usize, stop: Option<usize>, frames: &[Frame]) -> Option<usize> {
        let region = self.region_nodes(frames);
        let exits: HashSet<usize> = frames.iter().flat_map(|fr| [fr.brk, fr.cont]).flatten().collect();
        let reach = |from: usize| -> HashMap<usize, usize> {
            let mut distance = HashMap::new();
            let mut queue = std::collections::VecDeque::from([(from, 0usize)]);
            while let Some((v, d)) = queue.pop_front() {
                if v == b || exits.contains(&v) || region.is_some_and(|r| !r.contains(&v) && Some(v) != stop) || distance.contains_key(&v) {
                    continue;
                }
                distance.insert(v, d);
                if Some(v) != stop {
                    queue.extend(self.succs(v).into_iter().map(|s| (s, d + 1)));
                }
            }
            distance
        };
        let (from_t, from_f) = (reach(t), reach(f));
        from_t
            .iter()
            .filter_map(|(v, dt)| from_f.get(v).map(|df| (*v, dt + df)))
            .min_by_key(|(v, d)| (*d, self.blocks[*v].offset))
            .map(|(v, _)| v)
    }

    fn reaches(&self, from: usize, target: Option<usize>, avoid: usize, frames: &[Frame]) -> bool {
        let Some(target) = target else { return false };
        let exits: HashSet<usize> = frames.iter().flat_map(|f| [f.brk, f.cont]).flatten().collect();
        let mut seen = HashSet::from([avoid]);
        let mut work = vec![from];
        while let Some(v) = work.pop() {
            if v == target {
                return true;
            }
            if !seen.insert(v) || exits.contains(&v) {
                continue;
            }
            work.extend(self.succs(v));
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_if(&mut self, b: usize, c: Expr, t: usize, f: usize, stop: Option<usize>, frames: &mut Vec<Frame>, out: &mut Vec<Stmt>) -> Option<usize> {
        if t == f {
            // An empty if still evaluates its condition, which may run a getter.
            if !matches!(c.stripped(), Expr::Lit(_) | Expr::Local(_) | Expr::This) {
                out.push(Stmt::If(c, vec![], vec![]));
            }
            return Some(t);
        }
        let merge = self.merge_point(b, stop, frames).or_else(|| self.common_join(b, t, f, stop, frames));
        // The fallthrough branch is laid out first, so it reads as the `then` part.
        let (cond, first, second) = if self.blocks[f].offset <= self.blocks[t].offset {
            (negate(c.clone()), f, t)
        } else {
            (c.clone(), t, f)
        };
        match merge {
            Some(m) if m == second => {
                let mut then = Vec::new();
                self.region(first, Some(m), frames, &mut then);
                let s = self.nested_if(cond, then, out);
                out.push(s);
                Some(m)
            }
            Some(m) if m == first => {
                let mut then = Vec::new();
                self.region(second, Some(m), frames, &mut then);
                let s = self.nested_if(negate(cond), then, out);
                out.push(s);
                Some(m)
            }
            Some(m) => {
                let mut then = Vec::new();
                let mut otherwise = Vec::new();
                self.region(first, Some(m), frames, &mut then);
                self.region(second, Some(m), frames, &mut otherwise);
                out.push(Stmt::If(cond, then, otherwise));
                Some(m)
            }
            None => {
                // Neither branch returns here. One that cannot fall out of the region reads as an early exit.
                let first_out = self.reaches(first, stop, b, frames);
                let second_out = self.reaches(second, stop, b, frames);
                if !first_out || !second_out {
                    let (cond, exit, rest) = if !first_out { (cond, first, second) } else { (negate(cond), second, first) };
                    let mut then = Vec::new();
                    self.region(exit, stop, frames, &mut then);
                    let s = self.nested_if(cond, then, out);
                    out.push(s);
                    Some(rest)
                } else {
                    let mut then = Vec::new();
                    let mut otherwise = Vec::new();
                    self.region(first, stop, frames, &mut then);
                    self.region(second, stop, frames, &mut otherwise);
                    out.push(Stmt::If(cond, then, otherwise));
                    None
                }
            }
        }
    }

    /// `if(a && b)` without an else, where `a && b` was joined from two jumps, is how the
    /// compiler builds `if(a) { if(b) ... }`.
    fn nested_if(&self, cond: Expr, then: Vec<Stmt>, out: &mut Vec<Stmt>) -> Stmt {
        // A for-in whose body always leaves never loops back.
        if let Expr::HasNext(o, i) = cond.stripped() {
            return self.for_in(*o, *i, then, out);
        }
        let joined = self.compounds.iter().any(|k| *k == cond || negate(k.clone()) == cond || invert(k.clone()) == cond);
        match cond {
            Expr::Binary(BinOp::And, a, b) if joined => {
                let inner = self.nested_if(*b, then, &mut Vec::new());
                self.nested_if(*a, vec![inner], out)
            }
            Expr::Unary(UnOp::Not, inner) if joined && matches!(*inner, Expr::Binary(BinOp::Or, ..)) => {
                self.nested_if(invert(*inner), then, out)
            }
            cond => Stmt::If(cond, then, vec![]),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_switch(&mut self, b: usize, index: Expr, cases: Vec<usize>, default: usize, stop: Option<usize>, frames: &mut Vec<Frame>, out: &mut Vec<Stmt>) -> Option<usize> {
        let merge = self.merge_point(b, stop, frames);
        // Compilers test the subject against each case value and switch on the matching index.
        let chain = switch_chain(&index);
        let (subject, labels): (Expr, Vec<(usize, Option<Expr>)>) = match &chain {
            Some((subject, values, fallback)) => {
                let mut labels = Vec::new();
                for (value, i) in values {
                    let target = cases.get(*i as usize).copied().unwrap_or(default);
                    labels.push((target, Some(value.clone())));
                }
                let fallback_target = usize::try_from(*fallback).ok().and_then(|i| cases.get(i).copied()).unwrap_or(default);
                labels.push((fallback_target, None));
                (subject.clone(), labels)
            }
            None => {
                let mut labels: Vec<(usize, Option<Expr>)> = cases.iter().enumerate().map(|(i, t)| (*t, Some(Expr::int(i as i64)))).collect();
                labels.push((default, None));
                (index.clone(), labels)
            }
        };
        let subject = self.inline_temp(subject, out);
        // Every case returns: a default laid out after all of them is the code after the switch.
        let merge = merge.or_else(|| {
            let default_target = labels.iter().find(|(_, l)| l.is_none()).map(|(t, _)| *t)?;
            let after_all = labels
                .iter()
                .filter(|(_, l)| l.is_some())
                .all(|(t, _)| *t != default_target && self.blocks[*t].offset < self.blocks[default_target].offset);
            after_all.then_some(default_target)
        });
        let mut targets: Vec<usize> = labels.iter().map(|(t, _)| *t).filter(|t| Some(*t) != merge).collect();
        targets.sort_by_key(|t| self.blocks[*t].offset);
        targets.dedup();
        frames.push(Frame { brk: merge, cont: None, label: self.label(), used: false });
        let mut body = Vec::new();
        for (k, &target) in targets.iter().enumerate() {
            let case_labels: Vec<Option<Expr>> = labels.iter().filter(|(t, _)| *t == target).map(|(_, l)| l.clone()).collect();
            let next = targets.get(k + 1).copied().or(merge);
            let mut stmts = Vec::new();
            self.region(target, next, frames, &mut stmts);
            body.push((case_labels, stmts));
        }
        let frame = frames.pop().unwrap();
        let switch = Stmt::Switch(subject, body);
        out.push(if frame.used { Stmt::Labeled(frame.label, Box::new(switch)) } else { switch });
        merge
    }

    /// A switch subject saved to a register just before the dispatch reads better inline.
    fn inline_temp(&self, subject: Expr, out: &mut Vec<Stmt>) -> Expr {
        if let Expr::Local(r) = subject.stripped()
            && let Some(Stmt::Expr(Expr::Assign(target, value))) = out.last()
            && *target.stripped() == Expr::Local(*r)
        {
            let value = (**value).clone();
            out.pop();
            return value;
        }
        subject
    }

    fn label(&mut self) -> String {
        self.labels += 1;
        format!("loop{}", self.labels)
    }

    fn emit_loop(&mut self, header: usize, frames: &mut Vec<Frame>, out: &mut Vec<Stmt>) -> Option<usize> {
        self.entered.insert(header);
        let body = self.loops[&header].clone();
        let outside = |s: &usize| !body.contains(s);
        let preds = self.preds();
        let latches: Vec<usize> = preds[header].iter().copied().filter(|p| body.contains(p)).collect();
        let label = self.label();
        // while(cond): the header tests and leaves.
        if let Term::Cond(c, t, f) = self.blocks[header].term.clone()
            && self.blocks[header].stmts.is_empty()
            && (outside(&t) != outside(&f))
        {
            let (cond, inner, follow) = if outside(&f) { (c, t, f) } else { (negate(c), f, t) };
            self.done[header] = true;
            // A separate latch reached from several places is a for-loop's update, the continue target.
            let update = match latches.as_slice() {
                [l] if *l != inner && *l != header && preds[*l].len() > 1 && matches!(self.blocks[*l].term, Term::Goto(h) if h == header) => Some(*l),
                _ => None,
            };
            let cont = update.unwrap_or(header);
            frames.push(Frame { brk: Some(follow), cont: Some(cont), label: label.clone(), used: false });
            let mut stmts = Vec::new();
            self.region(inner, Some(cont), frames, &mut stmts);
            let frame = frames.pop().unwrap();
            let mut update_stmts = Vec::new();
            if let Some(l) = update {
                self.done[l] = true;
                update_stmts = self.blocks[l].stmts.clone();
            }
            strip_trailing_continue(&mut stmts);
            let looped = if let Expr::HasNext(o, i) = cond.stripped() {
                // for-in has no update clause: a latch with statements is the end of the body.
                stmts.extend(update_stmts);
                self.for_in(*o, *i, stmts, out)
            } else if update.is_some() && continues(&stmts, &label) {
                // `continue` runs the latch, so it is the loop's update clause.
                let init = match (out.last(), update_stmts.as_slice()) {
                    (Some(Stmt::Expr(Expr::Assign(target, _))), [Stmt::Expr(u)]) if assigns(u, target) => out.pop().map(Box::new),
                    _ => None,
                };
                Stmt::For(init, Some(cond), update_stmts, stmts)
            } else {
                // Otherwise the latch is the end of the body.
                stmts.extend(update_stmts);
                counted_loop(cond, stmts, out, &label)
            };
            out.push(if frame.used { Stmt::Labeled(frame.label, Box::new(looped)) } else { looped });
            return Some(follow);
        }
        // do { } while(cond): a latch tests and leaves.
        let tail = latches.iter().copied().find(|&l| {
            matches!(&self.blocks[l].term, Term::Cond(_, t, f) if (*t == header && outside(f)) || (*f == header && outside(t)))
        });
        if tail == Some(header) && latches.len() == 1 {
            // The whole loop is one block.
            let Term::Cond(c, t, f) = self.blocks[header].term.clone() else { unreachable!() };
            let (cond, follow) = if t == header { (c, f) } else { (negate(c), t) };
            self.done[header] = true;
            out.push(Stmt::DoWhile(self.blocks[header].stmts.clone(), cond));
            return Some(follow);
        }
        if let Some(l) = tail
            && latches.len() == 1
            && l != header
        {
            let Term::Cond(c, t, f) = self.blocks[l].term.clone() else { unreachable!() };
            let (cond, follow) = if t == header { (c, f) } else { (negate(c), t) };
            frames.push(Frame { brk: Some(follow), cont: Some(l), label: label.clone(), used: false });
            let mut stmts = Vec::new();
            self.region(header, Some(l), frames, &mut stmts);
            let frame = frames.pop().unwrap();
            self.done[l] = true;
            stmts.extend(self.blocks[l].stmts.clone());
            strip_trailing_continue(&mut stmts);
            let looped = Stmt::DoWhile(stmts, cond);
            out.push(if frame.used { Stmt::Labeled(frame.label, Box::new(looped)) } else { looped });
            return Some(follow);
        }
        // while(true) with breaks: the most common way out is the follow.
        let mut exits: HashMap<usize, usize> = HashMap::new();
        for &b in &body {
            for s in self.succs(b) {
                if outside(&s) {
                    *exits.entry(s).or_default() += 1;
                }
            }
        }
        let follow = exits.into_iter().max_by_key(|(s, n)| (*n, std::cmp::Reverse(self.blocks[*s].offset))).map(|(s, _)| s);
        frames.push(Frame { brk: follow, cont: Some(header), label, used: false });
        let mut stmts = Vec::new();
        self.region_from_header(header, frames, &mut stmts);
        let frame = frames.pop().unwrap();
        strip_trailing_continue(&mut stmts);
        let looped = Stmt::While(Expr::Lit(Lit::True), stmts);
        out.push(if frame.used { Stmt::Labeled(frame.label, Box::new(looped)) } else { looped });
        follow
    }

    /// Structures a loop body that starts at its own header.
    fn region_from_header(&mut self, header: usize, frames: &mut Vec<Frame>, out: &mut Vec<Stmt>) {
        // The header is the continue target, so the first visit must not read as `continue`.
        let saved = frames.last_mut().and_then(|f| f.cont.take());
        self.done[header] = true;
        out.extend(self.blocks[header].stmts.clone());
        if let Some(f) = frames.last_mut() {
            f.cont = saved;
        }
        let stop = None;
        match self.blocks[header].term.clone() {
            Term::Return(e) => out.push(Stmt::Return(e)),
            Term::Throw(e) => out.push(Stmt::Throw(e)),
            Term::End => {}
            Term::Goto(t) => self.region(t, Some(header), frames, out),
            Term::Cond(c, t, f) => {
                if let Some(n) = self.emit_if(header, c, t, f, Some(header), frames, out) {
                    self.region(n, Some(header), frames, out);
                }
            }
            Term::Switch(index, cases, default) => {
                if let Some(n) = self.emit_switch(header, index, cases, default, stop.or(Some(header)), frames, out) {
                    self.region(n, Some(header), frames, out);
                }
            }
        }
    }

    fn for_in(&self, object: u32, index: u32, mut stmts: Vec<Stmt>, out: &mut Vec<Stmt>) -> Stmt {
        // The loop variable's store may still be folded into its first use.
        hoist_assignments(&mut stmts);
        let mut each = false;
        let mut var = None;
        if let Some(Stmt::Expr(Expr::Assign(target, value))) = stmts.first() {
            match value.stripped() {
                Expr::NextName(o, i) | Expr::NextValue(o, i)
                    if *o.stripped() == Expr::Local(object) && *i.stripped() == Expr::Local(index) =>
                {
                    each = matches!(value.stripped(), Expr::NextValue(..));
                    var = Some((**target).clone());
                }
                _ => {}
            }
        }
        let Some(var) = var else {
            return Stmt::While(Expr::HasNext(object, index), stmts);
        };
        stmts.remove(0);
        if each && let Some(filter) = e4x_filter(object, index, &var, &stmts, out) {
            return filter;
        }
        // The compiler copies the object and zeroes the index just before the loop.
        let mut source = Expr::Local(object);
        for _ in 0..4 {
            match out.last() {
                Some(Stmt::Expr(Expr::Assign(target, value))) if *target.stripped() == Expr::Local(object) => {
                    source = (**value).clone();
                    out.pop();
                }
                Some(Stmt::Expr(Expr::Assign(target, _))) if *target.stripped() == Expr::Local(index) => {
                    out.pop();
                }
                // A compiler temporary that only carries the object here.
                Some(Stmt::Expr(Expr::Assign(target, value)))
                    if matches!(target.stripped(), Expr::Local(r) if self.killed.contains(r) && *source.stripped() == Expr::Local(*r)) =>
                {
                    source = (**value).clone();
                    out.pop();
                }
                _ => break,
            }
        }
        Stmt::ForIn { each, var, object: source, body: stmts }
    }

    /// Emits a try statement when `b` starts a protected range. `Some(next)` continues after it.
    fn try_region(&mut self, b: usize, _stop: Option<usize>, frames: &mut Vec<Frame>, out: &mut Vec<Stmt>) -> Option<Option<usize>> {
        let offset = self.blocks[b].offset as u32;
        let range = self
            .body
            .exceptions
            .iter()
            .filter(|e| e.from_offset == offset && !self.tries_done.contains(&(e.from_offset, e.to_offset)))
            .map(|e| (e.from_offset, e.to_offset))
            .max_by_key(|(_, to)| *to)?;
        self.tries_done.insert(range);
        let in_range = |o: usize| (o as u32) >= range.0 && (o as u32) < range.1;
        let handlers: Vec<usize> = self
            .body
            .exceptions
            .iter()
            .enumerate()
            .filter(|(_, e)| (e.from_offset, e.to_offset) == range)
            .map(|(n, _)| n)
            .collect();
        // The block normal flow reaches on leaving the protected range.
        let mut after = None;
        for (v, block) in self.blocks.iter().enumerate() {
            if !block.alive || !in_range(block.offset) {
                continue;
            }
            for s in block.term.succs() {
                if !in_range(self.blocks[s].offset) && self.blocks[s].handler.is_none() {
                    after = Some(after.map_or(s, |a: usize| if self.blocks[s].offset < self.blocks[a].offset { s } else { a }));
                }
            }
            let _ = v;
        }
        let mut body = Vec::new();
        self.region(b, after, frames, &mut body);
        let mut catches = Vec::new();
        for n in handlers {
            let target = self.body.exceptions[n].target_offset as usize;
            let Some(h) = self.blocks.iter().position(|blk| blk.alive && blk.offset == target) else { continue };
            let (name, kind) = self.catch_name(n);
            let mut stmts = Vec::new();
            self.region(h, after, frames, &mut stmts);
            catches.push(Catch { name, kind, body: stmts });
        }
        out.push(Stmt::Try(body, catches));
        Some(after)
    }
}

/// A local stored inside the first thing a statement evaluates reads better as its own
/// statement: `_loc4_ = new Object(); _loc4_.x = 1;` rather than `(_loc4_ = new Object()).x = 1;`.
/// Loop conditions keep theirs, since they run on every iteration.
fn hoist_assignments(stmts: &mut Vec<Stmt>) {
    let mut out = Vec::with_capacity(stmts.len());
    for mut s in stmts.drain(..) {
        let mut hoisted = Vec::new();
        // A register stored as it is returned is never read again.
        if let Stmt::Return(Some(e)) = &mut s
            && let Expr::Assign(target, value) = e
            && matches!(target.stripped(), Expr::Local(_))
        {
            *e = std::mem::replace(&mut **value, Expr::Global);
        }
        match &mut s {
            Stmt::Expr(e) => while let Some(h) = leftmost_store(e, true) { hoisted.push(h) },
            Stmt::Return(Some(e)) | Stmt::Throw(e) | Stmt::If(e, ..) | Stmt::Switch(e, _) => {
                while let Some(h) = leftmost_store(e, false) {
                    hoisted.push(h);
                }
            }
            _ => {}
        }
        match &mut s {
            Stmt::If(_, a, b) => {
                hoist_assignments(a);
                hoist_assignments(b);
            }
            Stmt::While(_, body) | Stmt::DoWhile(body, _) | Stmt::For(_, _, _, body) | Stmt::ForIn { body, .. } => hoist_assignments(body),
            Stmt::Switch(_, cases) => cases.iter_mut().for_each(|(_, body)| hoist_assignments(body)),
            Stmt::Labeled(_, inner) => {
                let mut one = vec![std::mem::replace(&mut **inner, Stmt::Break(None))];
                hoist_assignments(&mut one);
                **inner = one.pop().unwrap_or(Stmt::Break(None));
            }
            Stmt::Try(body, catches) => {
                hoist_assignments(body);
                catches.iter_mut().for_each(|c| hoist_assignments(&mut c.body));
            }
            _ => {}
        }
        // Innermost stores were found last but run first; each may hold stores of its own.
        let mut hoisted: Vec<Stmt> = hoisted.into_iter().rev().map(Stmt::Expr).collect();
        if !hoisted.is_empty() {
            hoist_assignments(&mut hoisted);
        }
        out.extend(hoisted);
        out.push(s);
    }
    *stmts = out;
}

/// Takes the local store `e` evaluates first, leaving the local in its place. `whole` is true
/// while `e` is the statement itself, which is never taken.
fn leftmost_store(e: &mut Expr, whole: bool) -> Option<Expr> {
    match e {
        Expr::Assign(target, value) if matches!(target.stripped(), Expr::Local(_)) => {
            if !whole {
                let local = target.stripped().clone();
                return Some(std::mem::replace(e, local));
            }
            // `a = b = c` stays a chain.
            if matches!(value.stripped(), Expr::Assign(..)) { None } else { leftmost_store(value, false) }
        }
        Expr::Assign(target, _) => match &mut **target {
            Expr::Prop(obj, _) => leftmost_store(obj, false),
            _ => None,
        },
        Expr::Coerce(_, inner) => leftmost_store(inner, whole),
        Expr::Prop(obj, _) | Expr::Binary(_, obj, _) | Expr::Unary(_, obj) | Expr::Ternary(obj, ..) => leftmost_store(obj, false),
        Expr::Call(callee, _) | Expr::New(callee, _) => leftmost_store(callee, false),
        _ => None,
    }
}

/// `res = src.(cond)` as compilers expand it: `res = new XMLList("")` and a for-each over `src`
/// that stores each item matching `cond` (read in a `with` scope of the item) into `res`.
fn e4x_filter(object: u32, index: u32, var: &Expr, body: &[Stmt], out: &mut Vec<Stmt>) -> Option<Stmt> {
    let mut rest = body.iter().filter(|s| !matches!(s, Stmt::Comment(c) if c.starts_with("with scope")));
    let mut cond = None;
    for s in rest.by_ref() {
        match s {
            Stmt::Expr(Expr::Assign(t, _)) if matches!(t.stripped(), Expr::Local(_)) && cond.is_none() => {}
            Stmt::If(c, then, otherwise) if otherwise.is_empty() && cond.is_none() => cond = Some((c, then)),
            _ => return None,
        }
    }
    let (cond, then) = cond?;
    let [Stmt::Expr(Expr::Assign(target, item))] = then.as_slice() else { return None };
    let Expr::Prop(result, Key::Index(_)) = target.stripped() else { return None };
    if item.stripped() != var.stripped() {
        return None;
    }
    // Before the loop: the index, the source copy and the result list, in any order.
    let mut source = None;
    let mut created = false;
    let mut taken = Vec::new();
    for (i, s) in out.iter().enumerate().rev().take(4) {
        let Stmt::Expr(Expr::Assign(t, v)) = s else { break };
        match t.stripped() {
            Expr::Local(r) if *r == object => source = Some((**v).clone()),
            Expr::Local(r) if *r == index => {}
            t if t == result.stripped() && matches!(v.stripped(), Expr::New(c, _) if *c.stripped() == Expr::Name("XMLList".into())) => created = true,
            _ => break,
        }
        taken.push(i);
    }
    let source = source?;
    if !created {
        return None;
    }
    for i in taken {
        out.remove(i);
    }
    Some(Stmt::Expr(Expr::Assign(result.clone(), Box::new(Expr::Filter(Box::new(source), Box::new(cond.clone()))))))
}

/// Applies `f` to every expression in `stmts`, as `Expr::map` does.
fn map_stmts(stmts: &mut [Stmt], f: &dyn Fn(&Expr) -> Option<Expr>) {
    for s in stmts.iter_mut() {
        match s {
            Stmt::Expr(e) | Stmt::Return(Some(e)) | Stmt::Throw(e) => *e = e.map(f),
            Stmt::If(c, a, b) => {
                *c = c.map(f);
                map_stmts(a, f);
                map_stmts(b, f);
            }
            Stmt::While(c, body) | Stmt::DoWhile(body, c) => {
                *c = c.map(f);
                map_stmts(body, f);
            }
            Stmt::For(init, cond, update, body) => {
                if let Some(init) = init {
                    map_stmts(std::slice::from_mut(&mut **init), f);
                }
                if let Some(c) = cond {
                    *c = c.map(f);
                }
                map_stmts(update, f);
                map_stmts(body, f);
            }
            Stmt::ForIn { var, object, body, .. } => {
                *var = var.map(f);
                *object = object.map(f);
                map_stmts(body, f);
            }
            Stmt::Switch(subject, cases) => {
                *subject = subject.map(f);
                for (labels, body) in cases.iter_mut() {
                    for l in labels.iter_mut().flatten() {
                        *l = l.map(f);
                    }
                    map_stmts(body, f);
                }
            }
            Stmt::Labeled(_, inner) => map_stmts(std::slice::from_mut(&mut **inner), f),
            Stmt::Try(body, catches) => {
                map_stmts(body, f);
                for c in catches.iter_mut() {
                    map_stmts(&mut c.body, f);
                }
            }
            Stmt::Return(None) | Stmt::Break(_) | Stmt::Continue(_) | Stmt::Comment(_) => {}
        }
    }
}

fn frames_innermost_loop(i: usize, last: usize) -> bool {
    // Frames between `i` and the innermost are switches only when they have no continue target;
    // the caller only reaches here for the frame that owns the target, so it is innermost when last.
    i == last
}

fn strip_trailing_continue(stmts: &mut Vec<Stmt>) {
    if matches!(stmts.last(), Some(Stmt::Continue(None))) {
        stmts.pop();
    }
}

/// `v = a; while(c(v)) { ...; v++; }` as `for(v = a; c(v); v++)`, unless a `continue` would
/// then also run the update.
fn counted_loop(cond: Expr, mut body: Vec<Stmt>, out: &mut Vec<Stmt>, label: &str) -> Stmt {
    if let (Some(Stmt::Expr(Expr::Assign(target, _))), Some(Stmt::Expr(update @ (Expr::Assign(..) | Expr::Update { .. })))) =
        (out.last(), body.last())
        && assigns(update, target)
        && !continues(&body, label)
    {
        let variable = target.stripped().clone();
        if cond.any(&|e| *e == variable) {
            let update = body.pop().into_iter().collect();
            let init = out.pop().map(Box::new);
            return Stmt::For(init, Some(cond), update, body);
        }
    }
    Stmt::While(cond, body)
}

/// Whether `stmts` continue the loop labelled `label` (directly, or by label from inner loops).
fn continues(stmts: &[Stmt], label: &str) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Continue(None) => true,
        Stmt::Continue(Some(l)) => l == label,
        Stmt::If(_, a, b) => continues(a, label) || continues(b, label),
        Stmt::Switch(_, cases) => cases.iter().any(|(_, body)| continues(body, label)),
        Stmt::Try(body, catches) => continues(body, label) || catches.iter().any(|c| continues(&c.body, label)),
        Stmt::Labeled(_, inner) => continues(std::slice::from_ref(&**inner), label),
        Stmt::While(_, body) | Stmt::DoWhile(body, _) | Stmt::For(_, _, _, body) | Stmt::ForIn { body, .. } => {
            labelled_continue(body, label)
        }
        _ => false,
    })
}

fn labelled_continue(stmts: &[Stmt], label: &str) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Continue(Some(l)) => l == label,
        Stmt::If(_, a, b) => labelled_continue(a, label) || labelled_continue(b, label),
        Stmt::Switch(_, cases) => cases.iter().any(|(_, body)| labelled_continue(body, label)),
        Stmt::Try(body, catches) => labelled_continue(body, label) || catches.iter().any(|c| labelled_continue(&c.body, label)),
        Stmt::Labeled(_, inner) => labelled_continue(std::slice::from_ref(&**inner), label),
        Stmt::While(_, body) | Stmt::DoWhile(body, _) | Stmt::For(_, _, _, body) | Stmt::ForIn { body, .. } => {
            labelled_continue(body, label)
        }
        _ => false,
    })
}

fn assigns(e: &Expr, target: &Expr) -> bool {
    match e {
        Expr::Assign(t, _) | Expr::Update { target: t, .. } => t.stripped() == target.stripped(),
        _ => false,
    }
}

fn pop(stack: &mut Vec<Val>) -> Result<Val> {
    stack.pop().ok_or_else(|| "operand stack underflow".to_string())
}

fn pop_n(stack: &mut Vec<Val>, n: u32) -> Result<Vec<Expr>> {
    let n = n as usize;
    if stack.len() < n {
        return Err("operand stack underflow".into());
    }
    Ok(stack.split_off(stack.len() - n).into_iter().map(|v| v.e).collect())
}

fn coerce(stack: &mut [Val], kind: String) -> Result<()> {
    let top = stack.last_mut().ok_or("coerce on an empty stack")?;
    let inner = match std::mem::replace(&mut top.e, Expr::Lit(Lit::Undefined)) {
        Expr::Coerce(_, inner) => *inner,
        other => other,
    };
    top.e = Expr::Coerce(kind, Box::new(inner));
    Ok(())
}

/// The item list of a vector literal being filled, if `e` is one and `slot` is its next index.
fn vector_items(e: &mut Expr, slot: i64) -> Option<&mut Vec<Expr>> {
    if slot == 0
        && let Expr::New(ctor, args) = e.stripped()
        && let Expr::TypeArgs(base, kinds) = ctor.stripped()
        && *base.stripped() == Expr::Name("Vector".into())
        && kinds.len() == 1
        && matches!(args.as_slice(), [n] if matches!(n.stripped(), Expr::Lit(Lit::Int(_))))
    {
        *e = Expr::Vector(Box::new(kinds[0].clone()), Vec::new());
    }
    match e {
        Expr::Vector(_, items) if items.len() as i64 == slot => Some(items),
        _ => None,
    }
}

fn pooled<T: Copy>(values: &[T], index: u32) -> Option<T> {
    index.checked_sub(1).and_then(|i| values.get(i as usize)).copied()
}

fn prop(obj: Expr, key: Key) -> Expr {
    match (&obj, &key) {
        (Expr::Scope(n), Key::Name(k)) if n == k => Expr::Name(k.clone()),
        (Expr::Global, Key::Name(k)) => Expr::Name(k.clone()),
        _ => Expr::Prop(Box::new(obj), key),
    }
}

/// `subject === v0 ? 0 : subject === v1 ? 1 : ... : k`, as compilers build a switch index.
fn switch_chain(index: &Expr) -> Option<(Expr, Vec<(Expr, i64)>, i64)> {
    let mut tests: Vec<(&Expr, &Expr, i64)> = Vec::new();
    let mut cur = index.stripped();
    let fallback = loop {
        match cur {
            Expr::Lit(Lit::Int(k)) => break *k,
            Expr::Ternary(c, a, b) => {
                let (x, y, hit, rest) = match c.stripped() {
                    Expr::Binary(BinOp::StrictEq, x, y) => (x, y, a, b),
                    Expr::Binary(BinOp::StrictNe, x, y) => (x, y, b, a),
                    _ => return None,
                };
                let Expr::Lit(Lit::Int(i)) = hit.stripped() else { return None };
                tests.push((x.stripped(), y.stripped(), *i));
                cur = rest.stripped();
            }
            _ => return None,
        }
    };
    let (first_x, first_y, _) = *tests.first()?;
    // The subject is the side every comparison shares; with one case, the non-constant side.
    let subject_is_x = if tests.iter().all(|(x, _, _)| *x == first_x) && tests.len() > 1 {
        true
    } else if tests.iter().all(|(_, y, _)| *y == first_y) && tests.len() > 1 {
        false
    } else if tests.len() == 1 {
        // Compilers push the case value first, so the subject is on the right.
        matches!(first_y, Expr::Lit(_)) || (matches!(first_x, Expr::Local(_)) && !matches!(first_y, Expr::Local(_)))
    } else {
        return None;
    };
    let subject = if subject_is_x { first_x.clone() } else { first_y.clone() };
    let values = tests.into_iter().map(|(x, y, i)| ((if subject_is_x { y } else { x }).clone(), i)).collect();
    Some((subject, values, fallback))
}

/// Immediate dominators by the iterative Cooper–Harvey–Kennedy method. `None` for unreachable
/// nodes and the roots (a root's entry is itself).
pub fn dominators(n: usize, roots: &[usize], succs: &dyn Fn(usize) -> Vec<usize>) -> Vec<Option<usize>> {
    // Reverse postorder from a virtual root that precedes every real root.
    let mut order = Vec::new();
    let mut seen = vec![false; n];
    for &root in roots {
        if seen[root] {
            continue;
        }
        let mut stack = vec![(root, 0usize)];
        seen[root] = true;
        while let Some((v, i)) = stack.pop() {
            let s = succs(v);
            if i < s.len() {
                stack.push((v, i + 1));
                if !seen[s[i]] {
                    seen[s[i]] = true;
                    stack.push((s[i], 0));
                }
            } else {
                order.push(v);
            }
        }
    }
    order.reverse();
    let virt = n;
    let mut rank = vec![usize::MAX; n + 1];
    rank[virt] = 0;
    for (i, &v) in order.iter().enumerate() {
        rank[v] = i + 1;
    }
    let mut preds = vec![Vec::new(); n];
    for &v in &order {
        for s in succs(v) {
            preds[s].push(v);
        }
    }
    let mut idom: Vec<Option<usize>> = vec![None; n + 1];
    idom[virt] = Some(virt);
    for &root in roots {
        idom[root] = Some(virt);
    }
    let intersect = |idom: &[Option<usize>], mut a: usize, mut b: usize| -> usize {
        while a != b {
            while rank[a] > rank[b] {
                a = idom[a].unwrap_or(virt);
            }
            while rank[b] > rank[a] {
                b = idom[b].unwrap_or(virt);
            }
        }
        a
    };
    let root_set: HashSet<usize> = roots.iter().copied().collect();
    let mut changed = true;
    while changed {
        changed = false;
        for &v in &order {
            if root_set.contains(&v) {
                continue;
            }
            let mut new: Option<usize> = None;
            for &p in &preds[v] {
                if idom[p].is_some() {
                    new = Some(match new {
                        None => p,
                        Some(cur) => intersect(&idom, p, cur),
                    });
                }
            }
            if new.is_some() && idom[v] != new {
                idom[v] = new;
                changed = true;
            }
        }
    }
    idom.truncate(n);
    idom.into_iter().map(|d| d.filter(|&d| d != virt)).collect()
}

fn dominates(idom: &[Option<usize>], a: usize, mut b: usize) -> bool {
    loop {
        if a == b {
            return true;
        }
        match idom[b] {
            Some(d) => b = d,
            None => return false,
        }
    }
}
