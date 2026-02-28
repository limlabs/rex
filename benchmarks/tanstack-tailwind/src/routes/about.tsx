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
    <div className="max-w-2xl mx-auto p-8">
      <h1 className="text-3xl font-bold text-gray-900 mb-4">About</h1>
      <p className="text-lg text-gray-600 mb-2">{description}</p>
      <p className="text-sm text-gray-400">Built at: {builtAt}</p>
      <Link to="/" className="mt-4 inline-block text-blue-600 hover:text-blue-800 underline">
        Back to home
      </Link>
    </div>
  )
}
