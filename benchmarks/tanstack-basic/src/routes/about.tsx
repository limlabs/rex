import { createFileRoute, Link } from '@tanstack/react-router'

export const Route = createFileRoute('/about')({
  loader: async () => {
    return {
      description: 'Rex is a Next.js Pages Router reimplemented in Rust.',
      builtAt: new Date().toISOString(),
    }
  },
  head: () => ({
    meta: [
      { title: 'About - Rex' },
      {
        name: 'description',
        content: 'Learn about Rex, a Next.js Pages Router in Rust.',
      },
    ],
  }),
  component: About,
})

function About() {
  const { description, builtAt } = Route.useLoaderData()
  return (
    <div>
      <h1>About</h1>
      <p>{description}</p>
      <p>Built at: {builtAt}</p>
      <Link to="/">Back to home</Link>
    </div>
  )
}
