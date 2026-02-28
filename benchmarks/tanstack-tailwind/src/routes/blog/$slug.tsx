import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/blog/$slug')({
  loader: async ({ params }) => {
    return {
      slug: params.slug,
      title: `Post about ${params.slug}`,
    }
  },
  component: BlogPost,
})

function BlogPost() {
  const { slug, title } = Route.useLoaderData()
  return (
    <div className="max-w-2xl mx-auto p-8">
      <h1 className="text-3xl font-bold text-gray-900 mb-4">Blog Post: {slug}</h1>
      <p className="text-gray-600">{title}</p>
    </div>
  )
}
