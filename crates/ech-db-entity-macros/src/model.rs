use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned, ToTokens};
use syn::{Attribute, Error, Expr, Fields, Ident, ItemEnum, Path, Type};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldKindModel {
    Plain,
    Map,
    Set,
    Feed,
}

impl FieldKindModel {
    pub fn is_collection(&self) -> bool {
        !matches!(self, FieldKindModel::Plain)
    }

    pub fn meta_variant(&self) -> TokenStream {
        match self {
            FieldKindModel::Plain => quote!(::ech_db_entity::FieldKind::Plain),
            FieldKindModel::Map => quote!(::ech_db_entity::FieldKind::Map),
            FieldKindModel::Set => quote!(::ech_db_entity::FieldKind::Set),
            FieldKindModel::Feed => quote!(::ech_db_entity::FieldKind::Feed),
        }
    }
}

pub enum FieldMigration {
    None,
    Default,
    Value(Expr),
}

pub struct FieldModel {
    pub ident: Ident,
    pub ty: Type,
    pub ty_key: String,
    pub kind: FieldKindModel,
    pub elements_key: Option<String>,
    pub migrate: FieldMigration,
}

pub struct VersionModel {
    pub ident: Ident,
    pub version: u32,
    pub fields: Vec<FieldModel>,
}

pub struct EntityModel {
    pub ident: Ident,
    pub event: Path,
    pub key: Vec<Ident>,
    pub versions: Vec<VersionModel>,
    pub transitions: Vec<Transition>,
    pub accessors: Vec<AccessorModel>,
}

pub fn type_key(ty: &Type) -> String {
    ty.to_token_stream().to_string()
}

pub fn collection_shape(ty: &Type) -> Option<(FieldKindModel, String)> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    let name = segment.ident.to_string();
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    let args: Vec<String> = arguments
        .args
        .iter()
        .map(|argument| argument.to_token_stream().to_string())
        .collect();
    let elements = args.join(",");
    match (name.as_str(), args.len()) {
        ("Map", 2) => Some((FieldKindModel::Map, elements)),
        ("Set", 1) => Some((FieldKindModel::Set, elements)),
        ("Feed", 1) => Some((FieldKindModel::Feed, elements)),
        _ => None,
    }
}

fn forbidden_attribute(attr: &Attribute, on: &str) -> Result<bool, Error> {
    let path = attr.path();
    if path.segments.last().map(|segment| segment.ident == "serde").unwrap_or(false) {
        return Err(Error::new_spanned(
            attr,
            "serde attributes are not allowed on entity types; the encoding is fixed",
        ));
    }
    if path.is_ident("doc") || path.is_ident("allow") {
        return Ok(true);
    }
    let _ = on;
    Ok(false)
}

fn filter_variant_attrs(attrs: &mut Vec<Attribute>) -> Result<(), Error> {
    let mut kept = Vec::new();
    for attr in attrs.drain(..) {
        if forbidden_attribute(&attr, "a version")? {
            kept.push(attr);
            continue;
        }
        return Err(Error::new_spanned(
            &attr,
            "attribute is not allowed on a version",
        ));
    }
    *attrs = kept;
    Ok(())
}

fn filter_field_attrs(attrs: &mut Vec<Attribute>) -> Result<FieldMigration, Error> {
    let mut migrate = FieldMigration::None;
    let mut seen_migrate = false;
    let mut kept = Vec::new();
    for attr in attrs.drain(..) {
        if attr.path().is_ident("migrate") {
            if seen_migrate {
                return Err(Error::new_spanned(&attr, "duplicate migrate attribute"));
            }
            seen_migrate = true;
            if matches!(attr.meta, syn::Meta::Path(_)) {
                migrate = FieldMigration::Default;
                continue;
            }
            let mut seen_default = false;
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("default") {
                    let value = meta.value()?;
                    let parsed: Expr = value.parse()?;
                    migrate = FieldMigration::Value(parsed);
                    seen_default = true;
                    Ok(())
                } else {
                    Err(meta.error("expected migrate(default = <expr>)"))
                }
            })?;
            if !seen_default {
                return Err(Error::new_spanned(
                    &attr,
                    "expected migrate(default = <expr>)",
                ));
            }
            continue;
        }
        if forbidden_attribute(&attr, "a field")? {
            kept.push(attr);
            continue;
        }
        return Err(Error::new_spanned(&attr, "attribute is not allowed on a field"));
    }
    *attrs = kept;
    Ok(migrate)
}

pub fn parse_entity_item(
    item: &mut ItemEnum,
    event: Path,
    key: Vec<Ident>,
) -> Result<EntityModel, Error> {
    if item.variants.is_empty() {
        return Err(Error::new_spanned(
            &*item,
            "entity needs at least one version",
        ));
    }
    let mut skip_derives = false;
    for attr in &item.attrs {
        if attr.path().is_ident("serde") {
            return Err(Error::new_spanned(
                attr,
                "serde attributes are not allowed on entity types; the encoding is fixed",
            ));
        }
        if attr.path().is_ident("derive") {
            let text = attr.to_token_stream().to_string();
            if text.contains("Serialize") || text.contains("Deserialize") {
                skip_derives = true;
            }
        }
    }
    if !skip_derives {
        item.attrs.push(syn::parse_quote!(
            #[derive(serde::Serialize, serde::Deserialize)]
        ));
    }
    let mut versions = Vec::new();
    for (index, variant) in item.variants.iter_mut().enumerate() {
        let version = index as u32 + 1;
        let expected = format!("V{version}");
        if variant.ident != expected {
            return Err(Error::new_spanned(
                &variant.ident,
                format!("entity versions must be named V1..Vn in order; expected {expected}"),
            ));
        }
        filter_variant_attrs(&mut variant.attrs)?;
        let Fields::Named(named) = &mut variant.fields else {
            return Err(Error::new_spanned(
                variant,
                "entity versions must have named fields",
            ));
        };
        let mut fields = Vec::new();
        for field in named.named.iter_mut() {
            let migrate = filter_field_attrs(&mut field.attrs)?;
            let ident = field.ident.clone().unwrap();
            let ty = field.ty.clone();
            let ty_key = type_key(&ty);
            let (kind, elements_key) = match collection_shape(&ty) {
                Some((kind, elements)) => (kind, Some(elements)),
                None => (FieldKindModel::Plain, None),
            };
            fields.push(FieldModel {
                ident,
                ty,
                ty_key,
                kind,
                elements_key,
                migrate,
            });
        }
        versions.push(VersionModel {
            ident: variant.ident.clone(),
            version,
            fields,
        });
    }
    for key_ident in &key {
        for version in &versions {
            let Some(field) = version.fields.iter().find(|field| field.ident == *key_ident) else {
                return Err(Error::new_spanned(
                    key_ident,
                    format!(
                        "key field `{key_ident}` is missing from version {}",
                        version.ident
                    ),
                ));
            };
            if field.kind.is_collection() {
                return Err(Error::new_spanned(
                    key_ident,
                    format!("key field `{key_ident}` cannot be a collection"),
                ));
            }
        }
    }
    for version in &versions {
        for key_ident in &key {
            let field = version
                .fields
                .iter()
                .find(|field| field.ident == *key_ident)
                .unwrap();
            let first = versions[0]
                .fields
                .iter()
                .find(|field| field.ident == *key_ident)
                .unwrap();
            if field.ty_key != first.ty_key {
                return Err(Error::new_spanned(
                    &field.ident,
                    format!(
                        "key field `{key_ident}` changed type in version {}",
                        version.ident
                    ),
                ));
            }
        }
    }
    let mut seen_key = Vec::new();
    for key_ident in &key {
        if seen_key.contains(&key_ident.to_string()) {
            return Err(Error::new_spanned(key_ident, "duplicate key field"));
        }
        seen_key.push(key_ident.to_string());
    }
    check_collections_across_versions(&versions)?;
    let transitions = check_transitions(&versions)?;
    let accessors = check_accessors(&versions)?;
    Ok(EntityModel {
        ident: item.ident.clone(),
        event,
        key,
        versions,
        transitions,
        accessors,
    })
}

pub fn check_collections_across_versions(versions: &[VersionModel]) -> Result<(), Error> {
    let mut history: Vec<(String, Vec<u32>, FieldKindModel, String)> = Vec::new();
    for version in versions {
        for field in &version.fields {
            if !field.kind.is_collection() {
                continue;
            }
            let name = field.ident.to_string();
            let elements = field.elements_key.clone().unwrap_or_default();
            match history.iter_mut().find(|(seen, _, _, _)| *seen == name) {
                Some((_, versions_present, kind, elements_seen)) => {
                    if *kind != field.kind || *elements_seen != elements {
                        return Err(Error::new_spanned(
                            &field.ident,
                            format!(
                                "collection `{}` changed kind or element types between versions",
                                field.ident
                            ),
                        ));
                    }
                    versions_present.push(version.version);
                }
                None => history.push((
                    name,
                    vec![version.version],
                    field.kind,
                    elements,
                )),
            }
        }
    }
    for (name, present, _, _) in history {
        let mut expected = present[0];
        for version in &present {
            if *version != expected {
                return Err(Error::new(
                    proc_macro2::Span::call_site(),
                    format!(
                        "collection `{name}` reappears after a gap in the version history"
                    ),
                ));
            }
            expected += 1;
        }
    }
    Ok(())
}

pub struct Transition {
    pub from: u32,
    pub old_patterns: Vec<TokenStream>,
    pub new_values: Vec<TokenStream>,
}

pub fn check_transitions(versions: &[VersionModel]) -> Result<Vec<Transition>, Error> {
    let mut transitions = Vec::new();
    for pair in versions.windows(2) {
        let old = &pair[0];
        let new = &pair[1];
        let mut old_patterns = Vec::new();
        for field in &old.fields {
            let used = if field.kind.is_collection() {
                new.fields
                    .iter()
                    .any(|candidate| candidate.kind.is_collection() && candidate.ident == field.ident)
            } else {
                new.fields
                    .iter()
                    .any(|candidate| candidate.kind == FieldKindModel::Plain && candidate.ident == field.ident)
            };
            let ident = &field.ident;
            if used {
                old_patterns.push(quote!(#ident));
            } else {
                old_patterns.push(quote!(#ident: _));
            }
        }
        let mut new_values = Vec::new();
        for field in &new.fields {
            if field.kind.is_collection() {
                match old.fields.iter().find(|candidate| {
                    candidate.kind.is_collection() && candidate.ident == field.ident
                }) {
                    Some(previous) => {
                        let ident = &previous.ident;
                        new_values.push(quote!(#ident));
                    }
                    None => match &field.migrate {
                        FieldMigration::None => {
                            return Err(Error::new_spanned(
                                &field.ident,
                                format!(
                                    "new field `{}` in {} needs #[migrate]",
                                    field.ident, new.ident
                                ),
                            ));
                        }
                        FieldMigration::Value(_) => {
                            return Err(Error::new_spanned(
                                &field.ident,
                                format!(
                                    "#[migrate(default = ...)] is not allowed on collection field `{}`",
                                    field.ident
                                ),
                            ));
                        }
                        FieldMigration::Default => {
                            let default = match field.kind {
                                FieldKindModel::Map => quote!(::ech_db_entity::Map::default()),
                                FieldKindModel::Set => quote!(::ech_db_entity::Set::default()),
                                FieldKindModel::Feed => quote!(::ech_db_entity::Feed::default()),
                                FieldKindModel::Plain => unreachable!(),
                            };
                            new_values.push(default);
                        }
                    },
                }
            } else {
                match old
                    .fields
                    .iter()
                    .find(|candidate| candidate.kind == FieldKindModel::Plain && candidate.ident == field.ident)
                {
                    Some(previous) => {
                        if previous.ty_key != field.ty_key {
                            return Err(Error::new_spanned(
                                &field.ident,
                                format!(
                                    "type of field `{}` changed between {} and {}",
                                    field.ident, old.ident, new.ident
                                ),
                            ));
                        }
                        let ident = &field.ident;
                        new_values.push(quote!(#ident));
                    }
                    None => match &field.migrate {
                        FieldMigration::None => {
                            return Err(Error::new_spanned(
                                &field.ident,
                                format!(
                                    "new field `{}` in {} needs #[migrate]",
                                    field.ident, new.ident
                                ),
                            ));
                        }
                        FieldMigration::Default => {
                            new_values.push(quote_spanned!(
                                field.ident.span() => ::core::default::Default::default()
                            ));
                        }
                        FieldMigration::Value(value) => new_values.push(quote!((#value))),
                    },
                }
            }
        }
        transitions.push(Transition {
            from: old.version,
            old_patterns,
            new_values,
        });
    }
    Ok(transitions)
}

pub struct AccessorModel {
    pub name: Ident,
    pub mut_name: Ident,
    pub ty: Type,
    pub ty_key: String,
    pub arms: Vec<(Ident, Ident)>,
    pub covered_all: bool,
    pub deprecated: bool,
}

const RESERVED_ACCESSOR_NAMES: &[&str] = &[
    "load",
    "load_root",
    "id",
    "spawn",
    "upgrade",
    "events",
    "apply",
    "version",
    "key_bytes",
    "is_genesis",
    "visit_collections",
    "migrate_step",
    "schema",
    "genesis",
    "body",
];

fn check_accessors(versions: &[VersionModel]) -> Result<Vec<AccessorModel>, Error> {
    let Some(latest) = versions.last() else {
        return Ok(Vec::new());
    };
    let mut accessors: Vec<AccessorModel> = Vec::new();
    for version in versions {
        for field in &version.fields {
            if let Some(index) = accessors
                .iter()
                .position(|accessor| accessor.name == field.ident)
            {
                let accessor = &mut accessors[index];
                if accessor.ty_key != field.ty_key {
                    return Err(Error::new_spanned(
                        &field.ident,
                        format!(
                            "field `{}` has different types in different versions; a single accessor cannot be generated",
                            field.ident
                        ),
                    ));
                }
                accessor.arms.push((version.ident.clone(), field.ident.clone()));
            } else {
                accessors.push(AccessorModel {
                    name: field.ident.clone(),
                    mut_name: format_ident!("{}_mut", field.ident),
                    ty: field.ty.clone(),
                    ty_key: field.ty_key.clone(),
                    arms: vec![(version.ident.clone(), field.ident.clone())],
                    covered_all: false,
                    deprecated: false,
                });
            }
        }
    }
    let mut candidates: Vec<(String, String)> = Vec::new();
    for accessor in accessors.iter_mut() {
        let name = accessor.name.to_string();
        if RESERVED_ACCESSOR_NAMES.contains(&name.as_str()) {
            return Err(Error::new_spanned(
                &accessor.name,
                format!("field `{name}` collides with a generated entity method"),
            ));
        }
        accessor.covered_all = accessor.arms.len() == versions.len();
        accessor.deprecated = !accessor
            .arms
            .iter()
            .any(|(variant, _)| *variant == latest.ident);
        for candidate in [name.clone(), accessor.mut_name.to_string()] {
            if let Some((_, owner)) = candidates.iter().find(|(seen, _)| *seen == candidate) {
                return Err(Error::new_spanned(
                    &accessor.name,
                    format!(
                        "field `{}` collides with the generated accessor `{candidate}` for field `{owner}`",
                        accessor.name
                    ),
                ));
            }
            candidates.push((candidate, name.clone()));
        }
    }
    Ok(accessors)
}
