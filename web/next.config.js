/** @type {import('next').NextConfig} */
const isStaticExport = process.env.NEXT_STATIC_EXPORT === '1';

const nextConfig = {
  // The desktop (Tauri) build produces a static export: no Node.js server runs
  // inside the WebView, so the whole app is emitted as HTML/CSS/JS. The web
  // deployment keeps the standalone Node server instead.
  output: isStaticExport ? 'export' : 'standalone',
  // Extend the dev proxy timeout so long-running chat requests (model API
  // calls plus tool loops) do not get ECONNRESET/socket hang up errors.
  experimental: {
    proxyTimeout: 300000, // 5 minutes
  },
};

// Rewrites require a running server, so they are unsupported in a static
// export. The desktop client instead talks to the control plane directly via
// NEXT_PUBLIC_RSMGO_CONTROL_URL (see web/lib/api.ts).
if (!isStaticExport) {
  nextConfig.rewrites = async () => {
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
  };
}

module.exports = nextConfig;
