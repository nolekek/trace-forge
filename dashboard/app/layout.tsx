import type { Metadata } from 'next';
import './globals.css';

export const metadata: Metadata = {
  title: 'TraceForge | Real-Time AgentOps Web Canvas & Control Center',
  description:
    'Sub-5ms Zero-Overhead Streaming Interceptor & Invariant Firewall for Autonomous AI Agents',
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en" className="dark h-full">
      <body className="h-full bg-slate-950 text-slate-100 antialiased font-sans flex flex-col">
        {children}
      </body>
    </html>
  );
}
