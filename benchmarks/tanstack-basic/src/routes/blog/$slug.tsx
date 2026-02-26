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
    <div>
      <h1>Blog Post: {slug}</h1>
      <p>{title}</p>
    </div>
  )
}
