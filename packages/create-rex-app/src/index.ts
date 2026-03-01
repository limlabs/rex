#!/usr/bin/env node

import fs from 'node:fs';
import path from 'node:path';
import { execSync } from 'node:child_process';
import readline from 'node:readline';

// --- Colors (ANSI) ---

const bold = (s: string): string => `\x1b[1m${s}\x1b[22m`;
const dim = (s: string): string => `\x1b[2m${s}\x1b[22m`;
const green = (s: string): string => `\x1b[32m${s}\x1b[39m`;
const greenBold = (s: string): string => bold(green(s));
const magenta = (s: string): string => `\x1b[35m${s}\x1b[39m`;
const magentaBold = (s: string): string => bold(magenta(s));
const red = (s: string): string => `\x1b[31m${s}\x1b[39m`;

// --- Prompts ---

function prompt(question: string): Promise<string> {
  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stderr,
  });
  return new Promise((resolve) => {
    rl.question(question, (answer) => {
      rl.close();
      resolve(answer.trim());
    });
  });
}

// --- Template files ---

const TEMPLATES: Record<string, Record<string, string>> = {
  default: {
    "pages/index.tsx": `export default function Home() {
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
`,
    "pages/_app.tsx": `import '../styles/globals.css';

export default function App({ Component, pageProps }: { Component: any; pageProps: any }) {
  return <Component {...pageProps} />;
}
`,
    "pages/api/hello.ts": `export default function handler(req: any, res: any) {
  res.status(200).json({ message: "Hello from Rex API!" });
}
`,
    "styles/globals.css": `*,
*::before,
*::after {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
}

body {
  font-family: system-ui, -apple-system, sans-serif;
  -webkit-font-smoothing: antialiased;
}

a {
  color: inherit;
  text-decoration: none;
}
`,
    tsconfig: `{
  "compilerOptions": {
    "target": "ESNext",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "jsx": "react-jsx",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true,
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true
  },
  "include": ["pages/**/*"],
  "exclude": ["node_modules", ".rex"]
}
`,
    gitignore: `node_modules
.rex
.DS_Store
`,
  },
};

// --- Detect package manager ---

type PackageManager = "npm" | "yarn" | "pnpm" | "bun";

function detectPackageManager(): PackageManager {
  const ua = process.env.npm_config_user_agent ?? "";
  if (ua.startsWith("pnpm")) return "pnpm";
  if (ua.startsWith("bun")) return "bun";
  if (ua.startsWith("yarn")) return "yarn";
  return "npm";
}

function installCommand(pm: PackageManager): string {
  return pm === "yarn" ? "yarn" : `${pm} install`;
}

function runCommand(pm: PackageManager): string {
  if (pm === "npm") return "npx";
  if (pm === "yarn") return "yarn";
  if (pm === "pnpm") return "pnpm exec";
  return "bunx";
}

// --- Main ---

async function main(): Promise<void> {
  let projectName = process.argv[2];

  console.error();
  console.error(`  ${magentaBold("◆ create-rex-app")}`);
  console.error();

  // Prompt for name if not provided
  if (!projectName) {
    projectName = await prompt(`  ${bold("Project name:")} `);
    if (!projectName) {
      console.error(`  ${red("✗")} Project name is required`);
      process.exit(1);
    }
    console.error();
  }

  // Validate name
  if (!/^[a-zA-Z0-9_-]+$/.test(projectName)) {
    console.error(
      `  ${red("✗")} Invalid project name: use only letters, numbers, hyphens, underscores`
    );
    process.exit(1);
  }

  const projectDir = path.resolve(projectName);

  if (fs.existsSync(projectDir)) {
    console.error(`  ${red("✗")} Directory '${projectName}' already exists`);
    process.exit(1);
  }

  const pm = detectPackageManager();
  const template = TEMPLATES.default;

  // Create directories
  fs.mkdirSync(path.join(projectDir, "pages", "api"), { recursive: true });
  fs.mkdirSync(path.join(projectDir, "styles"), { recursive: true });
  fs.mkdirSync(path.join(projectDir, "public"), { recursive: true });

  // Write package.json
  const packageJson = {
    name: projectName,
    version: "0.1.0",
    private: true,
    dependencies: {
      react: "^19.0.0",
      "react-dom": "^19.0.0",
    },
    devDependencies: {
      "@types/react": "^19.0.0",
      "@types/react-dom": "^19.0.0",
    },
  };
  fs.writeFileSync(
    path.join(projectDir, "package.json"),
    JSON.stringify(packageJson, null, 2) + "\n"
  );

  // Write template files
  for (const [filename, content] of Object.entries(template)) {
    let target = filename;
    if (filename === "tsconfig") target = "tsconfig.json";
    if (filename === "gitignore") target = ".gitignore";

    const filepath = path.join(projectDir, target);
    fs.mkdirSync(path.dirname(filepath), { recursive: true });
    fs.writeFileSync(filepath, content);
  }

  console.error(`  ${greenBold("✓")} Created ${bold(projectName)}`);
  console.error();

  // Install dependencies
  console.error(`  ${dim("Installing dependencies...")}`);
  console.error();

  try {
    execSync(installCommand(pm), {
      cwd: projectDir,
      stdio: "inherit",
    });
    console.error();
    console.error(`  ${greenBold("✓")} Dependencies installed`);
  } catch {
    console.error();
    console.error(
      `  ${dim("⚠ Could not install dependencies. Run")} ${bold(installCommand(pm))} ${dim("manually.")}`
    );
  }

  console.error();
  console.error(`  ${dim("Get started:")}`);
  console.error();
  console.error(`    ${bold("cd")} ${bold(projectName)}`);
  console.error(`    ${bold(`${runCommand(pm)} rex dev`)}`);
  console.error();
}

main().catch((err: Error) => {
  console.error(`  ${red("✗")} ${err.message}`);
  process.exit(1);
});
