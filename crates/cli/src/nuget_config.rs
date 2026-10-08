//! Edición de `NuGet.Config` (ADR-016): añade fuentes y `packageSourceMapping` conservando
//! todo lo existente (comentarios, espacios, otras fuentes) y nunca escribe secretos.
//!
//! Se usa un árbol mínimo que guarda el texto original de cada nodo, así que lo que no se
//! toca se reescribe byte a byte.

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Element(Element),
    /// Texto tal como estaba (con sus escapes).
    Text(String),
    /// Declaración, comentario, CDATA, instrucción o DOCTYPE, con sus delimitadores.
    Raw(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub name: String,
    /// Valores tal como estaban (con sus escapes).
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    pub self_closing: bool,
    /// Etiqueta de apertura original; se descarta al modificar el elemento.
    raw_open: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub nodes: Vec<Node>,
}

impl Element {
    fn new(name: &str, attrs: &[(&str, &str)]) -> Self {
        Self {
            name: name.to_owned(),
            attrs: attrs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), escape(v)))
                .collect(),
            children: Vec::new(),
            self_closing: true,
            raw_open: None,
        }
    }

    /// Valor de un atributo, sin escapes.
    pub fn attr(&self, name: &str) -> Option<String> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| unescape(v))
    }

    fn set_attr(&mut self, name: &str, value: &str) {
        let escaped = escape(value);
        match self.attrs.iter_mut().find(|(k, _)| k == name) {
            Some((_, v)) if unescape(v) == value => {}
            Some((_, v)) => {
                *v = escaped;
                self.raw_open = None;
            }
            None => {
                self.attrs.push((name.to_owned(), escaped));
                self.raw_open = None;
            }
        }
    }

    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    fn elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|e| e.name.eq_ignore_ascii_case(name))
    }

    fn child_mut(&mut self, name: &str) -> Option<&mut Element> {
        self.elements_mut()
            .find(|e| e.name.eq_ignore_ascii_case(name))
    }

    /// Añade un hijo con la sangría del documento, antes del espacio que precede al cierre.
    fn append(&mut self, child: Element, depth: usize, indent: &str) {
        let inner = format!("\n{}", indent.repeat(depth + 1));
        let outer = format!("\n{}", indent.repeat(depth));
        if self.self_closing
            || self
                .children
                .iter()
                .all(|n| matches!(n, Node::Text(t) if t.trim().is_empty()))
        {
            self.self_closing = false;
            self.raw_open = None;
            self.children = vec![Node::Text(inner), Node::Element(child), Node::Text(outer)];
            return;
        }
        let at = match self.children.last() {
            Some(Node::Text(t)) if t.trim().is_empty() => self.children.len() - 1,
            _ => {
                self.children.push(Node::Text(outer));
                self.children.len() - 1
            }
        };
        self.children.insert(at, Node::Element(child));
        self.children.insert(at, Node::Text(inner));
    }
}

fn escape(value: &str) -> String {
    quick_xml::escape::escape(value).into_owned()
}

fn unescape(value: &str) -> String {
    quick_xml::escape::unescape(value)
        .map(|v| v.into_owned())
        .unwrap_or_else(|_| value.to_owned())
}

fn element_from(e: &BytesStart<'_>, self_closing: bool) -> Result<Element, String> {
    let mut attrs = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|e| e.to_string())?;
        attrs.push((a.key.as_ref().to_owned(), a.value.clone().into_owned()));
    }
    let content: &str = e;
    let raw_open = if self_closing {
        format!("<{content}/>")
    } else {
        format!("<{content}>")
    };
    Ok(Element {
        name: e.name().as_ref().to_owned(),
        attrs,
        children: Vec::new(),
        self_closing,
        raw_open: Some(raw_open),
    })
}

impl Document {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut reader = Reader::from_str(text);
        reader.config_mut().trim_text(false);
        let mut stack: Vec<Element> = Vec::new();
        let mut top: Vec<Node> = Vec::new();
        let push =
            |stack: &mut Vec<Element>, top: &mut Vec<Node>, node: Node| match stack.last_mut() {
                Some(parent) => parent.children.push(node),
                None => top.push(node),
            };
        loop {
            let event = reader
                .read_event()
                .map_err(|e| format!("invalid XML at position {}: {e}", reader.error_position()))?;
            match event {
                Event::Start(e) => stack.push(element_from(&e, false)?),
                Event::Empty(e) => {
                    let el = element_from(&e, true)?;
                    push(&mut stack, &mut top, Node::Element(el));
                }
                Event::End(_) => {
                    let el = stack.pop().ok_or("closing tag without an opening tag")?;
                    push(&mut stack, &mut top, Node::Element(el));
                }
                Event::Text(t) => {
                    let raw = (*t).to_owned();
                    push(&mut stack, &mut top, Node::Text(raw));
                }
                Event::GeneralRef(r) => {
                    let raw = format!("&{};", &*r);
                    push(&mut stack, &mut top, Node::Text(raw));
                }
                Event::CData(t) => {
                    let raw = format!("<![CDATA[{}]]>", &*t);
                    push(&mut stack, &mut top, Node::Raw(raw));
                }
                Event::Comment(t) => {
                    let raw = format!("<!--{}-->", &*t);
                    push(&mut stack, &mut top, Node::Raw(raw));
                }
                Event::Decl(d) => {
                    let raw = format!("<?{}?>", &*d);
                    push(&mut stack, &mut top, Node::Raw(raw));
                }
                Event::PI(p) => {
                    let raw = format!("<?{}?>", &*p);
                    push(&mut stack, &mut top, Node::Raw(raw));
                }
                Event::DocType(d) => {
                    let raw = format!("<!DOCTYPE{}>", &*d);
                    push(&mut stack, &mut top, Node::Raw(raw));
                }
                Event::Eof => break,
            }
        }
        if !stack.is_empty() {
            return Err("unclosed elements".into());
        }
        Ok(Self { nodes: top })
    }

    pub fn new_config() -> Self {
        Self::parse(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<configuration>\n</configuration>\n",
        )
        .expect("valid template")
    }

    pub fn root(&self) -> Option<&Element> {
        self.nodes.iter().find_map(|n| match n {
            Node::Element(e) if e.name == "configuration" => Some(e),
            _ => None,
        })
    }

    fn root_mut(&mut self) -> Option<&mut Element> {
        self.nodes.iter_mut().find_map(|n| match n {
            Node::Element(e) if e.name == "configuration" => Some(e),
            _ => None,
        })
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for n in &self.nodes {
            render_node(n, &mut out);
        }
        out
    }

    /// Unidad de sangría del documento (la del primer hijo de `configuration`).
    fn indent(&self) -> String {
        self.root()
            .and_then(|r| {
                r.children.iter().find_map(|n| match n {
                    Node::Text(t) if t.contains('\n') => {
                        let unit = t.rsplit('\n').next().unwrap_or_default();
                        (!unit.is_empty() && unit.trim().is_empty()).then(|| unit.to_owned())
                    }
                    _ => None,
                })
            })
            .unwrap_or_else(|| "  ".to_owned())
    }

    /// Fuentes declaradas en este archivo: (clave, URL).
    pub fn sources(&self) -> Vec<(String, String)> {
        self.root()
            .and_then(|r| r.child("packageSources"))
            .map(|s| {
                s.elements()
                    .filter(|e| e.name == "add")
                    .filter_map(|e| Some((e.attr("key")?, e.attr("value")?)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `true` si `packageSources` empieza con `<clear />` (no hereda fuentes).
    pub fn clears_sources(&self) -> bool {
        self.root()
            .and_then(|r| r.child("packageSources"))
            .is_some_and(|s| s.elements().any(|e| e.name == "clear"))
    }

    /// Patrones de `packageSourceMapping` por fuente; `None` si no hay sección.
    pub fn mapping(&self) -> Option<Vec<(String, Vec<String>)>> {
        let section = self.root()?.child("packageSourceMapping")?;
        Some(
            section
                .elements()
                .filter(|e| e.name == "packageSource")
                .filter_map(|e| {
                    let patterns = e
                        .elements()
                        .filter(|p| p.name == "package")
                        .filter_map(|p| p.attr("pattern"))
                        .collect();
                    Some((e.attr("key")?, patterns))
                })
                .collect(),
        )
    }

    /// Claves con credenciales en texto plano (`ClearTextPassword`).
    pub fn cleartext_credentials(&self) -> Vec<String> {
        let Some(creds) = self
            .root()
            .and_then(|r| r.child("packageSourceCredentials"))
        else {
            return Vec::new();
        };
        creds
            .elements()
            .filter(|source| {
                source.elements().any(|e| {
                    e.attr("key")
                        .is_some_and(|k| k.eq_ignore_ascii_case("ClearTextPassword"))
                })
            })
            .map(|source| source.name.clone())
            .collect()
    }

    /// Añade o actualiza una fuente y su mapeo. Devuelve avisos para la persona.
    pub fn upsert_source(
        &mut self,
        key: &str,
        url: &str,
        patterns: &[String],
        allow_insecure: bool,
    ) -> Vec<String> {
        let mut notes = Vec::new();
        let indent = self.indent();
        let had_mapping = self.mapping().is_some();
        let clears = self.clears_sources();
        let others: Vec<String> = self
            .sources()
            .into_iter()
            .map(|(k, _)| k)
            .filter(|k| k != key)
            .collect();

        let root = self.root_mut().expect("the document has <configuration>");
        if root.child("packageSources").is_none() {
            root.append(Element::new("packageSources", &[]), 0, &indent);
        }
        let sources = root.child_mut("packageSources").expect("created above");
        let exists = sources
            .elements()
            .any(|e| e.name == "add" && e.attr("key").as_deref() == Some(key));
        if exists {
            let existing = sources
                .elements_mut()
                .find(|e| e.name == "add" && e.attr("key").as_deref() == Some(key))
                .expect("checked above");
            existing.set_attr("value", url);
            existing.set_attr("protocolVersion", "3");
            if allow_insecure {
                existing.set_attr("allowInsecureConnections", "true");
            }
        } else {
            let mut attrs = vec![("key", key), ("value", url), ("protocolVersion", "3")];
            if allow_insecure {
                attrs.push(("allowInsecureConnections", "true"));
            }
            sources.append(Element::new("add", &attrs), 1, &indent);
        }

        if root.child("packageSourceMapping").is_none() {
            root.append(Element::new("packageSourceMapping", &[]), 0, &indent);
        }
        let mapping = root
            .child_mut("packageSourceMapping")
            .expect("created above");
        if !had_mapping {
            // Con mapeo, NuGet solo usa para cada paquete las fuentes que lo mapean. Para no
            // romper lo que ya restauraba, el resto de fuentes conserva `*`; los patrones de
            // onepack son más específicos y ganan.
            let mut inherited = others.clone();
            if !clears && !inherited.iter().any(|k| k == "nuget.org") {
                inherited.push("nuget.org".to_owned());
                notes.push(
                    "this NuGet.Config inherits sources from other files: nuget.org is mapped to `*`; \
                     map any other inherited source by hand"
                        .to_owned(),
                );
            }
            for other in inherited {
                let mut source = Element::new("packageSource", &[("key", &other)]);
                source.append(Element::new("package", &[("pattern", "*")]), 2, &indent);
                mapping.append(source, 1, &indent);
            }
        }
        if !mapping
            .elements()
            .any(|e| e.name == "packageSource" && e.attr("key").as_deref() == Some(key))
        {
            mapping.append(Element::new("packageSource", &[("key", key)]), 1, &indent);
        }
        let ours = mapping
            .elements_mut()
            .find(|e| e.name == "packageSource" && e.attr("key").as_deref() == Some(key))
            .expect("created above");
        for pattern in patterns {
            if !ours
                .elements()
                .any(|p| p.name == "package" && p.attr("pattern").as_deref() == Some(pattern))
            {
                ours.append(Element::new("package", &[("pattern", pattern)]), 2, &indent);
            }
        }
        notes
    }
}

fn render_node(node: &Node, out: &mut String) {
    match node {
        Node::Text(t) | Node::Raw(t) => out.push_str(t),
        Node::Element(e) => {
            if let Some(raw) = &e.raw_open {
                out.push_str(raw);
                if !e.self_closing {
                    for c in &e.children {
                        render_node(c, out);
                    }
                    out.push_str(&format!("</{}>", e.name));
                }
                return;
            }
            out.push('<');
            out.push_str(&e.name);
            for (k, v) in &e.attrs {
                out.push_str(&format!(" {k}=\"{}\"", v.replace('"', "&quot;")));
            }
            if e.self_closing && e.children.is_empty() {
                out.push_str(" />");
                return;
            }
            out.push('>');
            for c in &e.children {
                render_node(c, out);
            }
            out.push_str(&format!("</{}>", e.name));
        }
    }
}

/// Clave de fuente para un feed: `onepack_<feed>` (los `-` pasan a `_` para que la variable
/// `NuGetPackageSourceCredentials_<clave>` sea válida en cualquier shell).
pub fn source_key(feed: &str) -> String {
    format!("onepack_{}", feed.replace('-', "_"))
}

/// Diff de líneas con dos líneas de contexto. Vacío si no hay cambios.
pub fn diff(old: &str, new: &str) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    // Tabla LCS: los archivos de configuración son pequeños.
    let mut lcs = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut ops: Vec<(char, &str)> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            ops.push((' ', a[i]));
            i += 1;
            j += 1;
        } else if j < b.len() && (i == a.len() || lcs[i][j + 1] >= lcs[i + 1][j]) {
            ops.push(('+', b[j]));
            j += 1;
        } else {
            ops.push(('-', a[i]));
            i += 1;
        }
    }
    if ops.iter().all(|(op, _)| *op == ' ') {
        return String::new();
    }
    const CONTEXT: usize = 2;
    let changed: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, (op, _))| *op != ' ')
        .map(|(i, _)| i)
        .collect();
    let mut out = Vec::new();
    let mut last_shown: Option<usize> = None;
    for (idx, (op, line)) in ops.iter().enumerate() {
        let near = changed
            .iter()
            .any(|&c| idx + CONTEXT >= c && idx <= c + CONTEXT);
        if !near {
            continue;
        }
        if last_shown.is_some_and(|l| idx > l + 1) {
            out.push("@@".to_owned());
        }
        out.push(format!("{op}{line}"));
        last_shown = Some(idx);
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXISTING: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<configuration>
    <!-- team sources -->
    <packageSources>
        <add key="nuget.org" value="https://api.nuget.org/v3/index.json" protocolVersion="3" />
        <add key="legacy" value="https://legacy.example/nuget?a=1&amp;b=2" />
    </packageSources>
    <config>
        <add key="globalPackagesFolder" value="packages" />
    </config>
</configuration>
"#;

    #[test]
    fn roundtrip_is_byte_identical() {
        for text in [
            EXISTING,
            "<configuration/>",
            "<a x='1' >t&amp;<![CDATA[x]]><b/></a>",
        ] {
            assert_eq!(Document::parse(text).unwrap().render(), text);
        }
    }

    #[test]
    fn adds_source_and_mapping_preserving_everything() {
        let mut doc = Document::parse(EXISTING).unwrap();
        let notes = doc.upsert_source(
            "onepack_internal",
            "https://packages.example.com/nuget/internal/v3/index.json",
            &["Hemia.*".into()],
            false,
        );
        let out = doc.render();
        assert!(out.contains("<!-- team sources -->"));
        assert!(out.contains("a=1&amp;b=2"));
        assert!(out.contains(r#"<add key="globalPackagesFolder" value="packages" />"#));
        assert!(out.contains(
            r#"        <add key="onepack_internal" value="https://packages.example.com/nuget/internal/v3/index.json" protocolVersion="3" />"#
        ));
        let mapping = doc.mapping().unwrap();
        assert_eq!(
            mapping,
            [
                ("nuget.org".to_owned(), vec!["*".to_owned()]),
                ("legacy".to_owned(), vec!["*".to_owned()]),
                ("onepack_internal".to_owned(), vec!["Hemia.*".to_owned()]),
            ]
        );
        assert_eq!(notes.len(), 0, "nuget.org was already declared");
        assert!(!out.contains("Password"));
        // Sigue siendo XML válido y con la sangría del archivo.
        assert!(Document::parse(&out).is_ok());
        assert!(out.contains("\n    <packageSourceMapping>\n        <packageSource key=\"nuget.org\">\n            <package pattern=\"*\" />"));
    }

    #[test]
    fn upsert_is_idempotent_and_merges_patterns() {
        let mut doc = Document::parse(EXISTING).unwrap();
        let url = "https://p.example/nuget/internal/v3/index.json";
        doc.upsert_source("onepack_internal", url, &["Hemia.*".into()], false);
        let once = doc.render();
        doc.upsert_source("onepack_internal", url, &["Hemia.*".into()], false);
        assert_eq!(doc.render(), once);
        doc.upsert_source("onepack_internal", url, &["Acme.*".into()], false);
        let mapping = doc.mapping().unwrap();
        assert_eq!(mapping.last().unwrap().1, ["Hemia.*", "Acme.*"]);
        assert_eq!(doc.sources().len(), 3);
    }

    #[test]
    fn new_file_inherits_nuget_org() {
        let mut doc = Document::new_config();
        let notes = doc.upsert_source(
            "onepack_x",
            "http://127.0.0.1:8080/nuget/x/v3/index.json",
            &["*".into()],
            true,
        );
        assert_eq!(notes.len(), 1);
        let out = doc.render();
        assert!(out.contains(r#"allowInsecureConnections="true""#));
        assert!(out.contains(r#"<packageSource key="nuget.org">"#));
        assert!(Document::parse(&out).unwrap().root().is_some());
    }

    #[test]
    fn detects_cleartext_credentials() {
        let doc = Document::parse(
            r#"<configuration><packageSourceCredentials><onepack_x><add key="Username" value="u" /><add key="ClearTextPassword" value="s" /></onepack_x></packageSourceCredentials></configuration>"#,
        )
        .unwrap();
        assert_eq!(doc.cleartext_credentials(), ["onepack_x"]);
    }

    #[test]
    fn source_keys() {
        assert_eq!(source_key("customer-a"), "onepack_customer_a");
    }

    #[test]
    fn line_diff() {
        assert_eq!(diff("a\nb\n", "a\nb\n"), "");
        let d = diff("1\n2\n3\n4\n5\n6\n7\n", "1\n2\n3\nX\n4\n5\n6\n7\n");
        assert_eq!(d, " 2\n 3\n+X\n 4\n 5");
    }
}
