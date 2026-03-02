// Rex Head - Client-side no-op
// On the server, Head collects elements for SSR.
// On the client, it renders nothing (head is already in the HTML).
export default function Head(): null {
  return null;
}
