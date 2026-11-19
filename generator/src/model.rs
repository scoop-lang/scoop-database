use crate::{Error, Result};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
pub struct Members {
    fields: Vec<Field>,
    properties: Vec<Property>,
    constructors: Vec<Constructor>,
}

#[derive(Deserialize)]
struct Field {
    declaration: Value,
    name: String,
    storage: Value,
    annotations: Vec<Annotation>,
    origin: Value,
}

#[derive(Deserialize)]
struct Property {
    declaration: Value,
    getter: Accessor,
    setter: Option<Accessor>,
    annotations: Vec<Annotation>,
}

#[derive(Deserialize)]
struct Accessor {
    access: Access,
    implementation: String,
}

impl Accessor {
    fn stored(&self) -> bool {
        matches!(self.implementation.as_str(), "storage" | "storageBody")
    }
}

#[derive(Deserialize)]
struct Access {
    accessible: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Constructor {
    constructor_kind: Value,
    access: Access,
    parameters: Vec<Parameter>,
    origin: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Parameter {
    name: String,
    target: Value,
    value_type: TypeUse,
    calling: Value,
    origin: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TypeUse {
    declaration_type: Value,
    applied_type: Value,
}

#[derive(Deserialize)]
struct Annotation {
    annotation: Value,
    arguments: Vec<Value>,
    origin: Value,
}

pub struct Column<'a> {
    pub name: &'a str,
    pub ty: &'a Value,
    pub origin: &'a Value,
    pub optional: bool,
    pub default: bool,
    pub config: Settings,
}

#[derive(Default)]
pub struct Settings {
    pub name: String,
    pub required: bool,
    pub emit_null: bool,
    pub ignore: bool,
}

impl Settings {
    fn read(annotations: &[Annotation], id: &Value, optional: bool) -> Result<Self> {
        let Some(item) = annotations.iter().find(|a| a.annotation == *id) else {
            return Ok(Self::default());
        };
        let [name, required, emit_null, ignore] = item.arguments.as_slice() else {
            return Err(Error::at(&item.origin, "invalid DbColumn arguments"));
        };
        let scalar = |v: &Value| v["value"]["value"].clone();
        let settings = Self {
            name: serde_json::from_value(scalar(name))?,
            required: serde_json::from_value(scalar(required))?,
            emit_null: serde_json::from_value(scalar(emit_null))?,
            ignore: serde_json::from_value(scalar(ignore))?,
        };
        if settings.ignore && (!settings.name.is_empty() || settings.emit_null || settings.required)
        {
            return Err(Error::at(
                &item.origin,
                "DbColumn ignore cannot be combined with other settings",
            ));
        }
        if settings.emit_null && !optional {
            return Err(Error::at(
                &item.origin,
                "DbColumn emitNull requires an Option field",
            ));
        }
        Ok(settings)
    }
}

impl Members {
    fn primary(&self) -> Option<&Constructor> {
        self.constructors
            .iter()
            .find(|c| c.constructor_kind["kind"] == "primary")
    }

    fn storage(&self, target: &Value) -> Option<(&Field, Option<&Property>)> {
        match target["kind"].as_str()? {
            "field" => {
                let field = self
                    .fields
                    .iter()
                    .find(|f| f.declaration == target["field"])?;
                Some((field, None))
            }
            "property" => {
                let id = &target["property"];
                let property = self.properties.iter().find(|p| p.declaration == *id)?;
                let field = self
                    .fields
                    .iter()
                    .find(|f| f.storage.get("property") == Some(id))?;
                Some((field, Some(property)))
            }
            _ => None,
        }
    }

    pub fn dependencies(&self) -> Vec<(String, Value)> {
        let Some(primary) = self.primary() else {
            return Vec::new();
        };
        primary
            .parameters
            .iter()
            .filter_map(|p| {
                let (_, property) = self.storage(&p.target)?;
                if property
                    .is_some_and(|prop| prop.setter.is_some() || !prop.getter.access.accessible)
                {
                    return None;
                }
                Some((p.name.clone(), p.value_type.applied_type.clone()))
            })
            .collect()
    }

    pub fn columns(
        &self,
        reading: bool,
        option: &Value,
        annotation: &Value,
    ) -> Result<Vec<Column<'_>>> {
        let primary = self
            .primary()
            .ok_or_else(|| Error::new("row generation requires a primary constructor"))?;
        if reading && !primary.access.accessible {
            return Err(Error::at(
                &primary.origin,
                "primary constructor is inaccessible to the row reader",
            ));
        }
        if reading
            && let Some(other) = self
                .constructors
                .iter()
                .find(|c| c.constructor_kind["kind"] == "secondary" && c.access.accessible)
        {
            return Err(Error::at(
                &other.origin,
                "row generation requires the primary constructor to be the only visible constructor",
            ));
        }
        let mut columns = Vec::new();
        let mut mapped = std::collections::HashSet::new();
        let mut names = std::collections::HashSet::new();
        for p in &primary.parameters {
            let optional = p.value_type.declaration_type["declaration"] == *option;
            let default = p
                .calling
                .get("default")
                .or_else(|| p.calling["omission"].get("default"))
                .is_some();
            let config = if let Some((field, property)) = self.storage(&p.target) {
                mapped.insert(&field.declaration);
                let annotations = property.map_or(&field.annotations, |prop| &prop.annotations);
                let config = Settings::read(annotations, annotation, optional)?;
                if !config.ignore
                    && (field.storage["kind"] == "propertyDelegate"
                        || property.is_some_and(|prop| {
                            !prop.getter.stored()
                                || prop.setter.as_ref().is_some_and(|s| !s.stored())
                                || (!reading && !prop.getter.access.accessible)
                        }))
                {
                    return Err(Error::at(
                        &p.origin,
                        format!("column `{}` requires ordinary accessible storage", p.name),
                    ));
                }
                config
            } else {
                Settings {
                    ignore: true,
                    ..Default::default()
                }
            };
            if config.ignore && reading && !optional && !default {
                return Err(Error::at(
                    &p.origin,
                    format!(
                        "omitted parameter `{}` requires a default or Option",
                        p.name
                    ),
                ));
            }
            let key = if config.name.is_empty() {
                &p.name
            } else {
                &config.name
            };
            if !config.ignore && !names.insert(key.clone()) {
                return Err(Error::at(
                    &p.origin,
                    format!("duplicate database column `{key}`"),
                ));
            }
            columns.push(Column {
                name: &p.name,
                ty: &p.value_type.applied_type,
                origin: &p.origin,
                default,
                optional,
                config,
            });
        }
        if let Some(field) = self
            .fields
            .iter()
            .find(|f| !mapped.contains(&f.declaration))
        {
            return Err(Error::at(
                &field.origin,
                format!(
                    "stored member `{}` is outside the primary constructor; provide a row mapping",
                    field.name
                ),
            ));
        }
        Ok(columns)
    }
}
