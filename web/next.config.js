/** @type {import('next').NextConfig} */
const nextConfig = {
  output: 'standalone',
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
