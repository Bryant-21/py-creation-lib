//! Move every class a movie defines into a private name space.
//!
//! Scaleform resolves a child movie's classes against the host menu first, so a
//! converted FO76 movie loaded into a stock FO4 menu silently runs FO4's
//! `ItemCard_StandardEntry`, `BSUIComponent` and even its own root class. Giving
//! the child unique names is the only way its own code runs.
//!
//! Packaged classes move under the prefix (`Shared.AS3.BSUIComponent` becomes
//! `B21.Shared.AS3.BSUIComponent`); classes in the root package keep the package
//! and take the prefix in their name (`ItemCard` becomes `B21_ItemCard`), because
//! the root package is also where the player's own built-ins live and the name
//! sets that resolve those classes cannot be extended safely.

use crate::abc::abc_block_offset;
use crate::container::{assemble, decompress, split_tags, tags_offset, write_tag_header};
use crate::symbolclass::{encode_symbol_table, parse_symbol_table};
use std::collections::{BTreeMap, BTreeSet};
use swf::avm2::{read::Reader, types::*, write::Writer};

type Result<T> = std::result::Result<T, String>;

const SYMBOL_CLASS: u16 = 76;

fn text(pool: &ConstantPool, index: Index<String>) -> String {
    if index.0 == 0 {
        String::new()
    } else {
        String::from_utf8_lossy(&pool.strings[index.0 as usize - 1]).into_owned()
    }
}

fn namespace_text(pool: &ConstantPool, index: Index<Namespace>) -> String {
    if index.0 == 0 {
        return String::new();
    }
    match pool.namespaces[index.0 as usize - 1] {
        Namespace::Namespace(s)
        | Namespace::Package(s)
        | Namespace::PackageInternal(s)
        | Namespace::Protected(s)
        | Namespace::Explicit(s)
        | Namespace::StaticProtected(s)
        | Namespace::Private(s) => text(pool, s),
    }
}

/// The namespace and name of a `QName`, which is how every class is declared.
fn qname(pool: &ConstantPool, index: Index<Multiname>) -> Option<(String, String)> {
    if index.0 == 0 {
        return None;
    }
    match pool.multinames[index.0 as usize - 1] {
        Multiname::QName { namespace, name } | Multiname::QNameA { namespace, name } => {
            Some((namespace_text(pool, namespace), text(pool, name)))
        }
        _ => None,
    }
}

fn trait_name(pool: &ConstantPool, index: Index<Multiname>) -> Option<String> {
    match pool.multinames.get(index.0.checked_sub(1)? as usize)? {
        Multiname::QName { name, .. }
        | Multiname::QNameA { name, .. }
        | Multiname::Multiname { name, .. }
        | Multiname::MultinameA { name, .. } => Some(text(pool, *name)),
        _ => None,
    }
}

#[derive(Default)]
struct Plan {
    packages: BTreeMap<String, String>,
    roots: BTreeMap<String, String>,
}

impl Plan {
    fn kept(qualified: &str, keep: &[String]) -> bool {
        keep.iter()
            .any(|k| qualified == k || qualified.starts_with(&format!("{k}.")))
    }

    fn collect(&mut self, abc: &AbcFile, prefix: &str, keep: &[String]) {
        for instance in &abc.instances {
            let Some((namespace, name)) = qname(&abc.constant_pool, instance.name) else {
                continue;
            };
            let qualified = if namespace.is_empty() {
                name.clone()
            } else {
                format!("{namespace}.{name}")
            };
            if Self::kept(&qualified, keep) {
                continue;
            }
            if namespace.is_empty() {
                self.roots.insert(name, String::new());
            } else {
                self.packages.insert(namespace, String::new());
            }
        }
        for (package, renamed) in &mut self.packages {
            *renamed = format!("{prefix}.{package}");
        }
        for (name, renamed) in &mut self.roots {
            *renamed = format!("{prefix}_{name}");
        }
    }

    /// A namespace URI is renamed whole when it is a package being moved, and by
    /// its class half when it is the protected namespace of one of its classes
    /// (`Shared.AS3:BSUIComponent`, or bare `ItemCard` in the root package).
    fn namespace(&self, uri: &str) -> Option<String> {
        if let Some(renamed) = self.packages.get(uri) {
            return Some(renamed.clone());
        }
        if let Some((package, class)) = uri.split_once(':') {
            if let Some(renamed) = self.packages.get(package) {
                return Some(format!("{renamed}:{class}"));
            }
        }
        self.roots.get(uri).cloned()
    }

    /// Guard against renaming a variable that happens to share a root class's
    /// name. Definitions and uses of a name move together, so a method may share
    /// one, but Flash assigns a timeline child to the variable its instance name
    /// matches, and that name lives in the art tags this pass does not touch.
    fn check_members(&self, abc: &AbcFile) -> Result<()> {
        let pool = &abc.constant_pool;
        let owners = abc.instances.iter().map(|i| {
            let owner = qname(pool, i.name).map_or(String::new(), |(_, n)| n);
            (owner, &i.traits)
        });
        let statics = abc.classes.iter().enumerate().map(|(index, c)| {
            let owner = abc
                .instances
                .get(index)
                .and_then(|i| qname(pool, i.name))
                .map_or(String::new(), |(_, n)| n);
            (format!("{owner} statics"), &c.traits)
        });
        for (owner, traits) in owners.chain(statics) {
            for member in traits {
                if !matches!(member.kind, TraitKind::Slot { .. } | TraitKind::Const { .. }) {
                    continue;
                }
                let Some(name) = trait_name(pool, member.name) else {
                    continue;
                };
                if self.roots.contains_key(&name) {
                    return Err(format!(
                        "Class {name} shares its name with a variable of {owner}, so renaming it could break a timeline binding"
                    ));
                }
            }
        }
        Ok(())
    }

    fn apply(&self, abc: &mut AbcFile) -> Result<()> {
        self.check_members(abc)?;
        let mut strings: BTreeMap<String, Index<String>> = BTreeMap::new();
        let mut intern = |pool: &mut ConstantPool, value: String| -> Index<String> {
            *strings.entry(value.clone()).or_insert_with(|| {
                pool.strings.push(value.into_bytes());
                Index::new(pool.strings.len() as u32)
            })
        };
        let pool = &mut abc.constant_pool;
        for index in 0..pool.namespaces.len() {
            let uri = namespace_text(pool, Index::new(index as u32 + 1));
            let Some(renamed) = self.namespace(&uri) else {
                continue;
            };
            let value = intern(pool, renamed);
            pool.namespaces[index] = match pool.namespaces[index] {
                Namespace::Namespace(_) => Namespace::Namespace(value),
                Namespace::Package(_) => Namespace::Package(value),
                Namespace::PackageInternal(_) => Namespace::PackageInternal(value),
                Namespace::Protected(_) => Namespace::Protected(value),
                Namespace::Explicit(_) => Namespace::Explicit(value),
                Namespace::StaticProtected(_) => Namespace::StaticProtected(value),
                Namespace::Private(_) => Namespace::Private(value),
            };
        }
        for index in 0..pool.multinames.len() {
            let name = match pool.multinames[index] {
                Multiname::QName { name, .. }
                | Multiname::QNameA { name, .. }
                | Multiname::Multiname { name, .. }
                | Multiname::MultinameA { name, .. } => name,
                _ => continue,
            };
            let Some(renamed) = self.roots.get(&text(pool, name)) else {
                continue;
            };
            let value = intern(pool, renamed.clone());
            match &mut pool.multinames[index] {
                Multiname::QName { name, .. }
                | Multiname::QNameA { name, .. }
                | Multiname::Multiname { name, .. }
                | Multiname::MultinameA { name, .. } => *name = value,
                _ => {}
            }
        }
        Ok(())
    }

    /// A SymbolClass entry names a class the way `getDefinitionByName` would.
    fn symbol(&self, bound: &str) -> Option<String> {
        if let Some(renamed) = self.roots.get(bound) {
            return Some(renamed.clone());
        }
        let (package, class) = bound.rsplit_once('.')?;
        self.packages
            .get(package)
            .map(|renamed| format!("{renamed}.{class}"))
    }
}

pub fn rename_classes(data: &[u8], prefix: &str, keep: &[String]) -> Result<Vec<u8>> {
    let movie = decompress(data)?;
    let mut plan = Plan::default();
    for span in split_tags(&movie.body)? {
        if !matches!(span.code, 72 | 82) {
            continue;
        }
        let payload = &movie.body[span.body_range()];
        let offset = abc_block_offset(span.code, payload);
        let abc = Reader::new(payload.get(offset..).ok_or("Truncated ABC header")?)
            .read()
            .map_err(|e| e.to_string())?;
        plan.collect(&abc, prefix, keep);
    }
    let renamed: BTreeSet<_> = plan
        .roots
        .keys()
        .chain(plan.packages.keys())
        .cloned()
        .collect();
    if renamed.is_empty() {
        return Err("The movie defines no classes to rename".into());
    }

    let mut output = movie.body[..tags_offset(&movie.body)?].to_vec();
    for span in split_tags(&movie.body)? {
        let payload = &movie.body[span.body_range()];
        if matches!(span.code, 72 | 82) {
            let offset = abc_block_offset(span.code, payload);
            let mut abc = Reader::new(payload.get(offset..).ok_or("Truncated ABC header")?)
                .read()
                .map_err(|e| e.to_string())?;
            plan.apply(&mut abc)?;
            let mut bytes = payload[..offset].to_vec();
            Writer::new(&mut bytes).write(abc).map_err(|e| e.to_string())?;
            output.extend(write_tag_header(span.code, bytes.len(), true));
            output.extend(bytes);
        } else if span.code == SYMBOL_CLASS {
            let mut entries = parse_symbol_table(payload)?;
            for entry in &mut entries {
                if let Some(renamed) = plan.symbol(&entry.name) {
                    entry.name = renamed;
                }
            }
            let table = encode_symbol_table(&entries);
            output.extend(write_tag_header(span.code, table.len(), true));
            output.extend(table);
        } else {
            output.extend_from_slice(&movie.body[span.start..span.end()]);
        }
    }
    assemble(movie.signature, movie.version, &output)
}
