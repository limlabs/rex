import { Inter } from 'next/font/google'

const inter = Inter({ weight: '400', subsets: ['latin'], display: 'swap' })

export default function Home() {
  return (
    <div className={inter.className}>
      <h1>Font Test - Pages Router</h1>
      <p style={inter.style}>This text uses the Inter font.</p>
    </div>
  )
}
