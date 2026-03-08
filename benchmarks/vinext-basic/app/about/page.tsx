export const metadata = {
  title: 'About - Rex',
  description: 'Learn about Rex, a Next.js Pages Router in Rust.',
};

export default function About() {
  const description = "Rex is a Next.js Pages Router reimplemented in Rust.";
  const builtAt = new Date().toISOString();

  return (
    <div>
      <h1>About</h1>
      <p>{description}</p>
      <p>Built at: {builtAt}</p>
      <a href="/">Back to home</a>
    </div>
  );
}
