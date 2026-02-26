import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/')({
  loader: async () => {
    return {
      message: 'Hello from Rex!',
      timestamp: new Date().toISOString(),
    }
  },
  component: Home,
})

function Home() {
  const { message, timestamp } = Route.useLoaderData()
  return (
    <div>
      <h1>Rex!</h1>
      <p>{message}</p>
      <p>Rendered at: {timestamp}</p>
    </div>
  )
}
