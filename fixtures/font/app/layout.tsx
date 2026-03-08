import { Roboto_Mono } from 'rex/font/google'
import React from 'react'

const mono = Roboto_Mono({
  weight: ['400', '700'],
  subsets: ['latin'],
  variable: '--font-mono',
})

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html className={mono.className}>
      <body>{children}</body>
    </html>
  )
}
