import { defineConfig } from 'vitepress';
import { withMermaid } from 'vitepress-plugin-mermaid';

export default withMermaid(
  defineConfig({
    title: 'pysafe-pickle',
    description:
      'Safe, fast, schema-evolvable Python object graph serialization powered by Rust.',
    base: '/pygraph/',
    lastUpdated: true,
    ignoreDeadLinks: false,
    head: [
      ['link', { rel: 'icon', type: 'image/svg+xml', href: '/pygraph/logo.svg' }],
      ['meta', { name: 'theme-color', content: '#10b981' }],
      ['meta', { property: 'og:title', content: 'pysafe-pickle Documentation' }],
      [
        'meta',
        {
          property: 'og:description',
          content: 'Safe, fast, schema-evolvable Python object graph serialization powered by Rust.',
        },
      ],
      ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
    ],
    appearance: 'dark',
    themeConfig: {
      logo: '/logo.svg',
      darkModeSwitchLabel: 'Appearance',
      lightModeSwitchTitle: 'Switch to light theme',
      darkModeSwitchTitle: 'Switch to dark theme',
      nav: [
        { text: 'Guide', link: '/guide/getting-started' },
        { text: 'API Reference', link: '/api/core' },
        { text: 'Architecture', link: '/architecture/binary-format' },
        { text: 'Benchmarks', link: '/benchmarks/performance' },
        {
          text: 'v1.1.0',
          items: [
            { text: 'PyPI Package', link: 'https://pypi.org/project/pysafe-pickle/' },
            { text: 'GitHub Releases', link: 'https://github.com/Abdullah-Masood-05/pygraph/releases' },
            { text: 'Repository', link: 'https://github.com/Abdullah-Masood-05/pygraph' },
          ],
        },
      ],
      sidebar: {
        '/guide/': [
          {
            text: 'Getting Started',
            items: [
              { text: 'What is pysafe-pickle?', link: '/guide/what-is-pysafe-pickle' },
              { text: 'Installation & Quickstart', link: '/guide/getting-started' },
              { text: 'Migration from pygraph', link: '/guide/migration-from-pygraph' },
            ],
          },
          {
            text: 'Core Features',
            items: [
              { text: 'Schema Evolution', link: '/guide/schema-evolution' },
              { text: 'Streaming & PickleBuffer', link: '/guide/streaming-and-buffers' },
              { text: 'Security & Allowlist', link: '/guide/security-and-allowlist' },
            ],
          },
        ],
        '/api/': [
          {
            text: 'API Reference',
            items: [
              { text: 'Core Functions (dumps/loads)', link: '/api/core' },
              { text: 'Streaming (Pickler/Unpickler)', link: '/api/streaming' },
              { text: 'Schema Migrations', link: '/api/migrations' },
              { text: 'Exceptions', link: '/api/exceptions' },
            ],
          },
        ],
        '/architecture/': [
          {
            text: 'Internals & Architecture',
            items: [
              { text: 'Binary Format Specification', link: '/architecture/binary-format' },
              { text: 'Rust Engine & PyO3', link: '/architecture/rust-engine' },
            ],
          },
        ],
        '/benchmarks/': [
          {
            text: 'Performance',
            items: [
              { text: 'Benchmarks vs Standard Pickle', link: '/benchmarks/performance' },
            ],
          },
        ],
      },
      socialLinks: [
        { icon: 'github', link: 'https://github.com/Abdullah-Masood-05/pygraph' },
      ],
      footer: {
        message: 'Released under the AGPL-3.0 License.',
        copyright: 'Copyright © 2026 Abdullah Masood',
      },
      search: { provider: 'local' },
    },
    mermaid: {
      theme: 'base',
      themeVariables: {
        darkMode: true,
        background: '#0f172a',
        primaryColor: '#1e293b',
        primaryBorderColor: '#334155',
        primaryTextColor: '#f8fafc',
        secondaryColor: '#0f766e',
        tertiaryColor: '#022c22',
        lineColor: '#94a3b8',
        textColor: '#e2e8f0',
        clusterBkg: '#022c22',
        clusterBorder: '#065f46',
        edgeLabelBackground: '#1e293b',
        nodeBorder: '#10b981',
        mainBkg: '#1e293b',
      },
    },
  }),
);
