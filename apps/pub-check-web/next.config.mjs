import { withBotId } from 'botid/next/config';

/** @type {import('next').NextConfig} */
const nextConfig = {
  poweredByHeader: false,
};

export default withBotId(nextConfig);
