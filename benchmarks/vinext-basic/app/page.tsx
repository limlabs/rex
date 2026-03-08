export const dynamic = 'force-dynamic';

export default function Home() {
  const message = "Hello from Rex!";
  const timestamp = new Date().toISOString();

  return (
    <div>
      <h1>Rex!</h1>
      <p>{message}</p>
      <p>Rendered at: {timestamp}</p>
    </div>
  );
}
