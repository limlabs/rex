use anyhow::Result;
use std::path::PathBuf;

use crate::display::*;

pub(crate) fn cmd_init(name: String) -> Result<()> {
    let project_dir = PathBuf::from(&name);

    if project_dir.exists() {
        anyhow::bail!("Directory '{}' already exists", name);
    }

    eprintln!();
    eprintln!("  {} {}", magenta_bold("◆ rex"), dim("creating project..."));
    eprintln!();

    // Create directory structure — no package.json needed.
    // Rex embeds React and extracts it automatically on first run.
    std::fs::create_dir_all(project_dir.join("pages"))?;
    std::fs::create_dir_all(project_dir.join("public"))?;

    // .gitignore
    std::fs::write(
        project_dir.join(".gitignore"),
        "node_modules\n.rex\n.DS_Store\n",
    )?;

    // pages/index.tsx
    std::fs::write(
        project_dir.join("pages/index.tsx"),
        r#"export default function Home() {
  return (
    <div style={{ fontFamily: "system-ui, sans-serif", padding: "2rem", maxWidth: "640px" }}>
      <h1>Welcome to Rex</h1>
      <p>Edit <code>pages/index.tsx</code> to get started.</p>
    </div>
  );
}

export async function getServerSideProps() {
  return {
    props: {
      createdAt: new Date().toISOString(),
    },
  };
}
"#,
    )?;

    eprintln!("  {} {}", green_bold("✓"), green_bold("Project created"));
    eprintln!();
    eprintln!("  {}", dim("Get started:"));
    eprintln!();
    eprintln!("    {} {}", bold("cd"), bold(&name));
    eprintln!("    {} {}", bold("rex dev"), dim(""));
    eprintln!();

    Ok(())
}
