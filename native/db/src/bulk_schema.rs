use std::collections::HashMap;

use serde::Deserialize;

use crate::error::{DbError, DbResult};
use crate::schema::validate_ident;

#[derive(Debug, Clone, Deserialize)]
pub struct SchemaDoc {
    pub tables: Vec<TableSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TableSpec {
    pub name: String,
    pub columns: Vec<String>,
    #[serde(default)]
    pub pk: Option<String>,
    #[serde(default)]
    pub on_conflict: Option<String>,
}

impl SchemaDoc {
    pub fn parse(json: &str) -> DbResult<Self> {
        let doc: SchemaDoc = serde_json::from_str(json)?;
        for t in &doc.tables {
            validate_ident(&t.name)?;
            if t.columns.is_empty() {
                return Err(DbError::Schema(format!(
                    "table '{}' has no columns",
                    t.name
                )));
            }
            for c in &t.columns {
                validate_ident(c)?;
            }
            if let Some(oc) = &t.on_conflict {
                match oc.to_ascii_uppercase().as_str() {
                    "REPLACE" | "IGNORE" | "ABORT" | "FAIL" | "ROLLBACK" => {}
                    other => {
                        return Err(DbError::Schema(format!(
                            "table '{}' has invalid on_conflict '{}'",
                            t.name, other
                        )));
                    }
                }
            }
        }
        Ok(doc)
    }

    pub fn by_name(&self) -> HashMap<String, &TableSpec> {
        self.tables.iter().map(|t| (t.name.clone(), t)).collect()
    }
}

impl TableSpec {
    /// Build the INSERT statement for this table. Includes the ON CONFLICT
    /// clause if configured.
    pub fn insert_sql(&self) -> String {
        let mut sql = String::from("INSERT ");
        if let Some(oc) = &self.on_conflict {
            sql.push_str(&format!("OR {} ", oc.to_ascii_uppercase()));
        }
        sql.push_str("INTO ");
        sql.push_str(&self.name);
        sql.push_str(" (");
        for (i, c) in self.columns.iter().enumerate() {
            if i > 0 {
                sql.push(',');
            }
            sql.push_str(c);
        }
        sql.push_str(") VALUES (");
        for i in 0..self.columns.len() {
            if i > 0 {
                sql.push(',');
            }
            sql.push('?');
        }
        sql.push(')');
        sql
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_builds_insert() {
        let doc = SchemaDoc::parse(
            r#"{"tables":[{"name":"nifs","columns":["id","name"],"on_conflict":"REPLACE"}]}"#,
        )
        .unwrap();
        let t = &doc.tables[0];
        assert_eq!(
            t.insert_sql(),
            "INSERT OR REPLACE INTO nifs (id,name) VALUES (?,?)"
        );
    }

    #[test]
    fn rejects_bad_ident() {
        let err = SchemaDoc::parse(r#"{"tables":[{"name":"bad;drop","columns":["id"]}]}"#);
        assert!(err.is_err());
    }
}
