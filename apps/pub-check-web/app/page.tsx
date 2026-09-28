import { cookies, headers } from 'next/headers';
import LandingClient from './LandingClient';
import { resolveCountry, resolveLocale } from '../lib/i18n';

export const dynamic = 'force-dynamic';

export default async function Home() {
  const [requestHeaders, cookieStore] = await Promise.all([headers(), cookies()]);
  const country = resolveCountry({
    cookie: cookieStore.get('chaptera-country')?.value,
    vercel: requestHeaders.get('x-vercel-ip-country'),
  });
  const locale = resolveLocale({
    cookie: cookieStore.get('chaptera-locale')?.value,
    accept: requestHeaders.get('accept-language'),
    country,
  });

  return <LandingClient initialCountry={country} initialLocale={locale} />;
}
