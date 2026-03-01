import { NextResponse } from 'rex/middleware'

export function middleware(request) {
  // Redirect /old-about to /about
  if (request.nextUrl.pathname === '/old-about') {
    return NextResponse.redirect(new URL('/about', request.url))
  }

  // Add a custom header on matched routes
  return NextResponse.next()
}

export const config = {
  matcher: ['/old-about', '/about', '/blog/:path*']
}
