//! MDX-to-JSX compiler.
//!
//! Compiles a single MDX source string into a JSX module by:
//! 1. Using OXC (error-recovering) to extract ESM imports/exports.
//! 2. Parsing the markdown+JSX body with the `markdown` crate's MDX AST.
//! 3. Walking the MDAST to generate `React.createElement` calls.
//!
//! Supports:
//! - Custom component overrides via `props.components` / `useMDXComponents`
//! - User `export default` as layout wrapper
//! - YAML frontmatter → `export const frontmatter = { ... }`
//! - Auto-import of `mdx-components.{tsx,ts,jsx,js}` when present

use anyhow::Result;
use markdown::mdast;
use std::collections::BTreeSet;

use crate::helpers::{
    extract_esm, jsx_string_literal, jsx_tag, mdx_attrs_to_props, yaml_to_js_object,
};

/// Options for MDX compilation.
#[derive(Default)]
pub struct MdxOptions {
    /// Absolute path to `mdx-components.{tsx,ts,jsx,js}` if found.
    pub mdx_components_path: Option<String>,
}

/// Compile a single MDX source string into a JSX module with options.
pub fn compile_mdx_with_options(source: &str, options: &MdxOptions) -> Result<String> {
    // Phase 1: Use OXC to find ESM statements at the top of the file.
    let (esm_lines, user_default_export, content_start) = extract_esm(source);

    // Phase 2: Parse the markdown/JSX body with the markdown crate.
    let body = &source[content_start..];
    let parse_opts = markdown::ParseOptions {
        constructs: markdown::Constructs {
            gfm_table: true,
            gfm_strikethrough: true,
            gfm_autolink_literal: true,
            gfm_footnote_definition: true,
            gfm_label_start_footnote: true,
            gfm_task_list_item: true,
            mdx_expression_flow: true,
            mdx_expression_text: true,
            mdx_jsx_flow: true,
            mdx_jsx_text: true,
            mdx_esm: true,
            frontmatter: true,
            html_flow: false,
            html_text: false,
            ..Default::default()
        },
        gfm_strikethrough_single_tilde: true,
        ..Default::default()
    };
    let ast = markdown::to_mdast(body, &parse_opts)
        .map_err(|e| anyhow::anyhow!("MDX parse error: {e}"))?;

    // Phase 3: Extract frontmatter and walk AST for createElement calls.
    let mut codegen = MdxCodegen::default();
    let mut frontmatter_yaml: Option<String> = None;
    let mut jsx_children = Vec::new();
    let mut body_esm_lines = Vec::new();

    if let mdast::Node::Root(root) = &ast {
        for child in &root.children {
            if let mdast::Node::Yaml(yaml) = child {
                frontmatter_yaml = Some(yaml.value.clone());
                continue;
            }
            // Collect ESM found by the markdown parser (e.g. imports after frontmatter
            // that extract_esm couldn't reach because frontmatter came first).
            if let mdast::Node::MdxjsEsm(esm) = child {
                body_esm_lines.push(esm.value.clone());
                continue;
            }
            if let Some(expr) = codegen.node_to_jsx(child) {
                jsx_children.push(expr);
            }
        }
    }

    // Phase 4: Assemble the output module.
    let mut output = String::new();

    // Emit extracted ESM imports/exports (from top-of-file and from markdown body)
    for line in &esm_lines {
        output.push_str(line);
        output.push('\n');
    }
    for line in &body_esm_lines {
        output.push_str(line);
        output.push('\n');
    }

    // Emit frontmatter export if present
    if let Some(yaml) = &frontmatter_yaml {
        let js_obj = yaml_to_js_object(yaml);
        output.push_str(&format!("export const frontmatter = {js_obj};\n"));
    }

    // Import createElement
    output.push_str("import { createElement } from 'react';\n");

    // Import useMDXComponents if mdx-components file exists
    if let Some(mdx_components_path) = &options.mdx_components_path {
        output.push_str(&format!(
            "import {{ useMDXComponents as _provideComponents }} from '{mdx_components_path}';\n"
        ));
    }
    output.push('\n');

    // Build _components defaults from used HTML tags
    let defaults = codegen.component_defaults();

    // Generate the content component
    output.push_str("function MDXContent(props) {\n");
    if options.mdx_components_path.is_some() {
        output.push_str(&format!(
            "  const _components = {{ {defaults}..._provideComponents(), ...props.components }};\n"
        ));
    } else {
        output.push_str(&format!(
            "  const _components = {{ {defaults}...props.components }};\n"
        ));
    }
    output.push_str("  return createElement('div', { className: '_mdx-content' },\n");
    for (i, child) in jsx_children.iter().enumerate() {
        output.push_str("    ");
        output.push_str(child);
        if i < jsx_children.len() - 1 {
            output.push(',');
        }
        output.push('\n');
    }
    output.push_str("  );\n");
    output.push_str("}\n\n");

    // Export: wrap with user layout if provided, otherwise export MDXContent directly
    if let Some(ref layout_expr) = user_default_export {
        output.push_str(&format!("const _userLayout = {layout_expr};\n"));
        output.push_str(
            "export default function MDXWrapper(props) {\n\
             \x20 return createElement(_userLayout, props, createElement(MDXContent, props));\n\
             }\n",
        );
    } else {
        output.push_str("export default MDXContent;\n");
    }

    Ok(output)
}

/// State accumulated during code generation.
#[derive(Default)]
struct MdxCodegen {
    /// HTML tags used in the document (for _components defaults).
    used_tags: BTreeSet<String>,
}

impl MdxCodegen {
    /// Build the defaults portion of `_components = { h1: 'h1', ... }`.
    fn component_defaults(&self) -> String {
        if self.used_tags.is_empty() {
            return String::new();
        }
        let pairs: Vec<String> = self
            .used_tags
            .iter()
            .map(|tag| format!("{tag}: '{tag}'"))
            .collect();
        format!("{}, ", pairs.join(", "))
    }

    /// Record an HTML tag and return the `_components.tag` expression.
    fn html_tag(&mut self, tag: &str) -> String {
        self.used_tags.insert(tag.to_string());
        format!("_components.{tag}")
    }

    /// Convert an MDAST node to a `createElement` expression string.
    fn node_to_jsx(&mut self, node: &mdast::Node) -> Option<String> {
        match node {
            mdast::Node::Heading(h) => {
                let tag = format!("h{}", h.depth);
                let tag_expr = self.html_tag(&tag);
                let children = self.children_to_jsx(&h.children);
                Some(format!("createElement({tag_expr}, null, {children})"))
            }
            mdast::Node::Paragraph(p) => {
                let tag = self.html_tag("p");
                let children = self.children_to_jsx(&p.children);
                Some(format!("createElement({tag}, null, {children})"))
            }
            mdast::Node::Text(t) => Some(jsx_string_literal(&t.value)),
            mdast::Node::Strong(s) => {
                let tag = self.html_tag("strong");
                let children = self.children_to_jsx(&s.children);
                Some(format!("createElement({tag}, null, {children})"))
            }
            mdast::Node::Emphasis(e) => {
                let tag = self.html_tag("em");
                let children = self.children_to_jsx(&e.children);
                Some(format!("createElement({tag}, null, {children})"))
            }
            mdast::Node::Delete(d) => {
                let tag = self.html_tag("del");
                let children = self.children_to_jsx(&d.children);
                Some(format!("createElement({tag}, null, {children})"))
            }
            mdast::Node::InlineCode(c) => {
                let tag = self.html_tag("code");
                let val = jsx_string_literal(&c.value);
                Some(format!("createElement({tag}, null, {val})"))
            }
            mdast::Node::Code(c) => {
                let pre_tag = self.html_tag("pre");
                let code_tag = self.html_tag("code");
                let val = jsx_string_literal(&c.value);
                let code_props = c
                    .lang
                    .as_ref()
                    .map(|l| format!("{{ className: 'language-{l}' }}"))
                    .unwrap_or_else(|| "null".to_string());
                Some(format!(
                    "createElement({pre_tag}, null, createElement({code_tag}, {code_props}, {val}))"
                ))
            }
            mdast::Node::Link(l) => {
                let tag = self.html_tag("a");
                let children = self.children_to_jsx(&l.children);
                let title_prop = l
                    .title
                    .as_ref()
                    .map(|t| format!(", title: {}", jsx_string_literal(t)))
                    .unwrap_or_default();
                let href = jsx_string_literal(&l.url);
                Some(format!(
                    "createElement({tag}, {{ href: {href}{title_prop} }}, {children})"
                ))
            }
            mdast::Node::Image(img) => {
                let tag = self.html_tag("img");
                let src = jsx_string_literal(&img.url);
                let alt = jsx_string_literal(&img.alt);
                let title_prop = img
                    .title
                    .as_ref()
                    .map(|t| format!(", title: {}", jsx_string_literal(t)))
                    .unwrap_or_default();
                Some(format!(
                    "createElement({tag}, {{ src: {src}, alt: {alt}{title_prop} }})"
                ))
            }
            mdast::Node::List(list) => {
                let tag_name = if list.ordered { "ol" } else { "ul" };
                let tag = self.html_tag(tag_name);
                let props = if list.ordered {
                    list.start
                        .filter(|&s| s != 1)
                        .map(|s| format!("{{ start: {s} }}"))
                        .unwrap_or_else(|| "null".to_string())
                } else {
                    "null".to_string()
                };
                let items: Vec<String> = list
                    .children
                    .iter()
                    .filter_map(|c| self.node_to_jsx(c))
                    .collect();
                Some(format!(
                    "createElement({tag}, {props}, {})",
                    items.join(", ")
                ))
            }
            mdast::Node::ListItem(li) => {
                let tag = self.html_tag("li");
                let children: Vec<String> = li
                    .children
                    .iter()
                    .filter_map(|c| {
                        if let mdast::Node::Paragraph(p) = c {
                            Some(self.children_to_jsx(&p.children))
                        } else {
                            self.node_to_jsx(c)
                        }
                    })
                    .collect();
                Some(format!(
                    "createElement({tag}, null, {})",
                    children.join(", ")
                ))
            }
            mdast::Node::Blockquote(bq) => {
                let tag = self.html_tag("blockquote");
                let children: Vec<String> = bq
                    .children
                    .iter()
                    .filter_map(|c| self.node_to_jsx(c))
                    .collect();
                Some(format!(
                    "createElement({tag}, null, {})",
                    children.join(", ")
                ))
            }
            mdast::Node::ThematicBreak(_) => {
                let tag = self.html_tag("hr");
                Some(format!("createElement({tag}, null)"))
            }
            mdast::Node::Break(_) => {
                let tag = self.html_tag("br");
                Some(format!("createElement({tag}, null)"))
            }
            mdast::Node::Html(h) => {
                let tag = self.html_tag("div");
                let val = jsx_string_literal(&h.value);
                Some(format!(
                    "createElement({tag}, {{ dangerouslySetInnerHTML: {{ __html: {val} }} }})"
                ))
            }
            mdast::Node::Table(t) => Some(self.table_to_jsx(t)),
            mdast::Node::MdxJsxFlowElement(el) => Some(self.mdx_jsx_flow_to_jsx(el)),
            mdast::Node::MdxJsxTextElement(el) => Some(self.mdx_jsx_text_to_jsx(el)),
            mdast::Node::MdxTextExpression(expr) => Some(expr.value.clone()),
            mdast::Node::MdxFlowExpression(expr) => Some(expr.value.clone()),
            mdast::Node::MdxjsEsm(_) => None, // Handled in compile_mdx_with_options
            mdast::Node::Yaml(_) => None,     // Handled in compile_mdx_with_options
            _ => None,
        }
    }

    /// Convert inline children to a comma-separated list of createElement args.
    fn children_to_jsx(&mut self, children: &[mdast::Node]) -> String {
        let parts: Vec<String> = children
            .iter()
            .filter_map(|c| self.node_to_jsx(c))
            .collect();
        if parts.is_empty() {
            "null".to_string()
        } else {
            parts.join(", ")
        }
    }

    /// Convert an MDX JSX flow element to a createElement call.
    fn mdx_jsx_flow_to_jsx(&mut self, el: &mdast::MdxJsxFlowElement) -> String {
        let tag = jsx_tag(&el.name);
        let props = mdx_attrs_to_props(&el.attributes);
        let children: Vec<String> = el
            .children
            .iter()
            .filter_map(|c| self.node_to_jsx(c))
            .collect();
        if children.is_empty() {
            format!("createElement({tag}, {props})")
        } else {
            format!("createElement({tag}, {props}, {})", children.join(", "))
        }
    }

    /// Convert an MDX JSX text element to a createElement call.
    fn mdx_jsx_text_to_jsx(&mut self, el: &mdast::MdxJsxTextElement) -> String {
        let tag = jsx_tag(&el.name);
        let props = mdx_attrs_to_props(&el.attributes);
        let children: Vec<String> = el
            .children
            .iter()
            .filter_map(|c| self.node_to_jsx(c))
            .collect();
        if children.is_empty() {
            format!("createElement({tag}, {props})")
        } else {
            format!("createElement({tag}, {props}, {})", children.join(", "))
        }
    }

    /// Convert an MDAST table to a createElement call.
    fn table_to_jsx(&mut self, table: &mdast::Table) -> String {
        let table_tag = self.html_tag("table");
        let thead_tag = self.html_tag("thead");
        let tbody_tag = self.html_tag("tbody");
        let tr_tag = self.html_tag("tr");
        let align = &table.align;
        let mut rows = Vec::new();

        for (row_idx, child) in table.children.iter().enumerate() {
            if let mdast::Node::TableRow(row) = child {
                let cell_tag_name = if row_idx == 0 { "th" } else { "td" };
                let cell_tag = self.html_tag(cell_tag_name);
                let mut cells = Vec::new();
                for (col_idx, cell_node) in row.children.iter().enumerate() {
                    if let mdast::Node::TableCell(cell) = cell_node {
                        let content = self.children_to_jsx(&cell.children);
                        let style = align
                            .get(col_idx)
                            .and_then(|a| match a {
                                mdast::AlignKind::Left => {
                                    Some("{ style: { textAlign: 'left' } }".to_string())
                                }
                                mdast::AlignKind::Center => {
                                    Some("{ style: { textAlign: 'center' } }".to_string())
                                }
                                mdast::AlignKind::Right => {
                                    Some("{ style: { textAlign: 'right' } }".to_string())
                                }
                                mdast::AlignKind::None => None,
                            })
                            .unwrap_or_else(|| "null".to_string());
                        cells.push(format!("createElement({cell_tag}, {style}, {content})"));
                    }
                }
                let row_expr = format!("createElement({tr_tag}, null, {})", cells.join(", "));
                rows.push(row_expr);
            }
        }

        let (head_rows, body_rows) = if !rows.is_empty() {
            (vec![rows[0].clone()], rows[1..].to_vec())
        } else {
            (vec![], vec![])
        };

        let thead = if !head_rows.is_empty() {
            format!("createElement({thead_tag}, null, {})", head_rows.join(", "))
        } else {
            "null".to_string()
        };
        let tbody = if !body_rows.is_empty() {
            format!("createElement({tbody_tag}, null, {})", body_rows.join(", "))
        } else {
            "null".to_string()
        };

        format!("createElement({table_tag}, null, {thead}, {tbody})")
    }
}
