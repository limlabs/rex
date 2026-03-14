use super::*;

#[test]
fn extracts_arrow_function_in_jsx() {
    let source = r#"import { sendMessage } from "./utils"

export default function ContactForm() {
  return (
    <form action={async (data: FormData) => {
      "use server"
      const name = data.get("name")
      sendMessage(name)
    }}>
      <input name="name" />
    </form>
  )
}"#;

    let result = extract_inline_server_actions(source, Path::new("test.tsx")).unwrap();
    assert!(
        result
            .source
            .contains("export async function __rex_action_0"),
        "Should hoist the function. Got:\n{}",
        result.source
    );
    assert!(
        result.source.contains("action={__rex_action_0}"),
        "Should replace inline with reference. Got:\n{}",
        result.source
    );
    assert!(
        !result.source.contains("\"use server\""),
        "Should remove directive. Got:\n{}",
        result.source
    );
    assert_eq!(result.actions.len(), 1);
    assert_eq!(result.actions[0].name, "__rex_action_0");
}

#[test]
fn skips_module_level_use_server() {
    let source = r#""use server"

export async function myAction() {
    return 42
}
"#;
    let result = extract_inline_server_actions(source, Path::new("test.ts"));
    assert!(
        result.is_none(),
        "Module-level 'use server' should be skipped"
    );
}

#[test]
fn skips_file_without_use_server() {
    let source = r#"export default function Page() {
    return <div>Hello</div>
}
"#;
    let result = extract_inline_server_actions(source, Path::new("test.tsx"));
    assert!(result.is_none());
}

#[test]
fn handles_single_quote_directive() {
    let source = r#"export default function Form() {
  return (
    <form action={async (fd) => {
      'use server'
      console.log(fd)
    }}>
      <button type="submit">Go</button>
    </form>
  )
}"#;

    let result = extract_inline_server_actions(source, Path::new("test.tsx")).unwrap();
    assert!(result.source.contains("__rex_action_0"));
    assert!(!result.source.contains("'use server'"));
}

#[test]
fn preserves_function_params() {
    let source = r#"export default function Form() {
  return (
    <form action={async (data: FormData) => {
      "use server"
      const x = data.get("x")
    }}>
      <button>Submit</button>
    </form>
  )
}"#;

    let result = extract_inline_server_actions(source, Path::new("test.tsx")).unwrap();
    assert!(
        result.actions[0].source.contains("(data: FormData)"),
        "Should preserve params. Got: {}",
        result.actions[0].source
    );
}
