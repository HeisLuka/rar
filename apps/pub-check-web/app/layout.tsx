import type { Metadata } from 'next';
import './globals.css';

export const metadata: Metadata = {
  title: 'Chaptera PUB Check',
  description:
    'Upload a Microsoft Publisher .pub file and receive a compatibility report by email.',
};

export default function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
