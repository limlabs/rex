use anyhow::Result;
use swc_common::{comments::SingleThreadedComments, sync::Lrc, FileName, Mark, GLOBALS, SourceMap};
use swc_ecma_ast::*;
use swc_ecma_codegen::{text_writer::JsWriter, Emitter};
use swc_ecma_parser::{lexer::Lexer, Parser, StringInput, Syntax, TsSyntax};
use swc_ecma_transforms_base::{fixer::fixer, hygiene::hygiene, resolver};
use swc_ecma_transforms_react::{self as react_transform, Runtime};
use swc_ecma_transforms_typescript::strip;

/// Transform options
#[derive(Debug, Clone)]
pub struct TransformOptions {
    /// Whether this is a server-side transform (keeps getServerSideProps)
    pub server: bool,
    /// Whether to enable React Fast Refresh (dev mode)
    pub fast_refresh: bool,
    /// Whether the source is TypeScript
    pub typescript: bool,
    /// Whether the source contains JSX
    pub jsx: bool,
}

impl Default for TransformOptions {
    fn default() -> Self {
        Self {
            server: false,
            fast_refresh: false,
            typescript: true,
            jsx: true,
        }
    }
}

/// Transform a single source file using SWC
pub fn transform_file(source: &str, filename: &str, opts: &TransformOptions) -> Result<String> {
    let cm: Lrc<SourceMap> = Lrc::new(SourceMap::default());
    let fm = cm.new_source_file(FileName::Real(filename.into()).into(), source.to_string());
    let comments = SingleThreadedComments::default();

    let syntax = if opts.typescript || opts.jsx {
        Syntax::Typescript(TsSyntax {
            tsx: opts.jsx,
            decorators: false,
            ..Default::default()
        })
    } else {
        Syntax::Es(Default::default())
    };

    let lexer = Lexer::new(syntax, EsVersion::Es2022, StringInput::from(&*fm), Some(&comments));
    let mut parser = Parser::new_from(lexer);
    let module = parser
        .parse_module()
        .map_err(|e| anyhow::anyhow!("Parse error in {}: {:?}", filename, e))?;

    GLOBALS.set(&Default::default(), || {
        let unresolved_mark = Mark::new();
        let top_level_mark = Mark::new();

        // Convert to Program for Pass-based transforms
        let mut program = Program::Module(module);

        // 1. Resolver
        resolver(unresolved_mark, top_level_mark, opts.typescript).process(&mut program);

        // 2. Strip TypeScript
        if opts.typescript {
            strip(unresolved_mark, top_level_mark).process(&mut program);
        }

        // 3. JSX transform
        // Server bundles use Classic runtime (React.createElement) since they run
        // in V8 as scripts with React available globally.
        // Client bundles use Automatic runtime (jsx-runtime imports).
        if opts.jsx {
            let jsx_opts = react_transform::Options {
                runtime: Some(if opts.server {
                    Runtime::Classic
                } else {
                    Runtime::Automatic
                }),
                ..Default::default()
            };
            react_transform::react(
                cm.clone(),
                Some(&comments),
                jsx_opts,
                top_level_mark,
                unresolved_mark,
            )
            .process(&mut program);
        }

        // 4. Hygiene + fixer
        hygiene().process(&mut program);
        fixer(Some(&comments)).process(&mut program);

        // Extract module back
        let module = match program {
            Program::Module(m) => m,
            _ => unreachable!(),
        };

        // 5. Strip getServerSideProps for client bundles
        let module = if !opts.server {
            strip_gssp(module)
        } else {
            module
        };

        // Emit
        let mut buf = Vec::new();
        {
            let writer = JsWriter::new(cm.clone(), "\n", &mut buf, None);
            let mut emitter = Emitter {
                cfg: swc_ecma_codegen::Config::default().with_minify(false),
                cm: cm.clone(),
                comments: Some(&comments),
                wr: writer,
            };
            emitter.emit_module(&module)?;
        }

        Ok(String::from_utf8(buf)?)
    })
}

/// Remove `getServerSideProps` export from a module (for client bundles)
fn strip_gssp(mut module: Module) -> Module {
    module.body.retain(|item| {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export_decl)) => {
                match &export_decl.decl {
                    Decl::Fn(fn_decl) => fn_decl.ident.sym.as_ref() != "getServerSideProps",
                    Decl::Var(var_decl) => {
                        !var_decl.decls.iter().any(|d| {
                            matches!(&d.name, Pat::Ident(ident) if ident.sym.as_ref() == "getServerSideProps")
                        })
                    }
                    _ => true,
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(named)) => {
                let has_gssp = named.specifiers.iter().any(|s| {
                    if let ExportSpecifier::Named(n) = s {
                        let name = match &n.orig {
                            ModuleExportName::Ident(i) => i.sym.as_ref(),
                            ModuleExportName::Str(s) => {
                                // Wtf8Atom - use as_str for comparison
                                return s.value.as_str().map_or(false, |v| v == "getServerSideProps");
                            }
                        };
                        name == "getServerSideProps"
                    } else {
                        false
                    }
                });
                if has_gssp {
                    named.specifiers.len() > 1
                } else {
                    true
                }
            }
            _ => true,
        }
    });
    module
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transform_basic_tsx() {
        let source = r#"
            import React from 'react';

            interface Props {
                name: string;
            }

            export default function Home({ name }: Props) {
                return <div>Hello {name}</div>;
            }

            export async function getServerSideProps() {
                return { props: { name: "World" } };
            }
        "#;

        // Server transform keeps GSSP
        let server_result = transform_file(source, "index.tsx", &TransformOptions {
            server: true,
            ..Default::default()
        }).unwrap();
        assert!(server_result.contains("getServerSideProps"));

        // Client transform strips GSSP
        let client_result = transform_file(source, "index.tsx", &TransformOptions {
            server: false,
            ..Default::default()
        }).unwrap();
        assert!(!client_result.contains("getServerSideProps"));
    }
}
