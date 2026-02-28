import { createRootRoute, Link, Outlet } from '@tanstack/react-router'
import '../styles/globals.css'

export const Route = createRootRoute({
  component: RootComponent,
})

function RootComponent() {
  return (
    <div className="min-h-screen bg-gray-50">
      <nav className="bg-white shadow-sm border-b border-gray-200 px-6 py-3 flex gap-4">
        <Link to="/">Home</Link>
        <Link to="/about">About</Link>
      </nav>
      <main className="max-w-4xl mx-auto px-6 py-8">
        <Outlet />
      </main>
    </div>
  )
}
