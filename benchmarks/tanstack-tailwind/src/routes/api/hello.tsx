import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const getHello = createServerFn({ method: 'GET' }).handler(async () => {
  return { message: 'Hello from Rex API!', method: 'GET' }
})

export const Route = createFileRoute('/api/hello')({
  loader: async () => {
    return await getHello()
  },
  component: ApiHello,
})

function ApiHello() {
  const data = Route.useLoaderData()
  return <pre className="p-4 bg-gray-100 rounded">{JSON.stringify(data, null, 2)}</pre>
}
