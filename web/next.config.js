/** @type {import('next').NextConfig} */
const nextConfig = {
  output: 'standalone',
  // Extend the dev proxy timeout so long-running chat requests (model API
  // calls plus tool loops) do not get ECONNRESET/socket hang up errors.
  experimental: {
    proxyTimeout: 300000, // 5 minutes
  },
  async rewrites() {
    const controlUrl = process.env.RSMGO_CONTROL_URL || 'http://localhost:9090';
    return [
      {
        source: '/api/:path*',
        destination: `${controlUrl}/api/:path*`,
      },
      {
        source: '/health',
        destination: `${controlUrl}/health`,
      },
    ];
  },
};

module.exports = nextConfig;
