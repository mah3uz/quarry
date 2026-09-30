// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
	integrations: [
		starlight({
			title: 'quarry',
			description:
				'A fast, beautiful SQL client for the terminal: a REPL and a full-screen TUI for PostgreSQL, MySQL / MariaDB and SQLite.',
			logo: { src: './src/assets/logo.svg', alt: 'quarry' },
			favicon: '/favicon.svg',
			customCss: [
				'@fontsource-variable/inter',
				'./src/styles/theme.css',
			],
			expressiveCode: {
				themes: ['tokyo-night', 'github-light'],
				styleOverrides: { borderRadius: '0.6rem' },
			},
			sidebar: [
				{
					label: 'Start here',
					items: [
						{ label: 'What is quarry?', slug: 'start/introduction' },
						{ label: 'Installation', slug: 'start/installation' },
						{ label: 'Quick start', slug: 'start/quick-start' },
					],
				},
				{
					label: 'Guides',
					items: [
						{ label: 'Connecting to a database', slug: 'guides/connecting' },
						{ label: 'Using the REPL', slug: 'guides/repl' },
						{ label: 'Using the TUI', slug: 'guides/tui' },
						{ label: 'Browsing and editing tables', slug: 'guides/editing-data' },
						{ label: 'Scripts, exports and pipes', slug: 'guides/scripting' },
						{ label: 'Asking a model for SQL', slug: 'guides/ai' },
						{ label: 'Staying safe', slug: 'guides/safety' },
					],
				},
				{
					label: 'Advanced',
					items: [
						{ label: 'Saved connections', slug: 'advanced/saved-connections' },
						{ label: 'Passwords and secrets', slug: 'advanced/passwords' },
						{ label: 'TLS and SSH tunnels', slug: 'advanced/tls-ssh' },
						{ label: 'Favourite queries', slug: 'advanced/favorites' },
						{ label: 'Themes', slug: 'advanced/themes' },
						{ label: 'Prompt, keys and completion', slug: 'advanced/customising' },
					],
				},
				{
					label: 'Reference',
					items: [
						{ label: 'Command-line options', slug: 'reference/cli' },
						{ label: 'Special commands', slug: 'reference/special-commands' },
						{ label: 'Keyboard shortcuts', slug: 'reference/keys' },
						{ label: 'Configuration file', slug: 'reference/config' },
						{ label: 'Output formats', slug: 'reference/output-formats' },
						{ label: 'Environment variables', slug: 'reference/environment' },
						{ label: 'Files and directories', slug: 'reference/files' },
					],
				},
				{
					label: 'Help',
					items: [
						{ label: 'Troubleshooting', slug: 'help/troubleshooting' },
						{ label: 'Coming from pgcli, mycli or litecli', slug: 'help/migrating' },
						{ label: 'Contributing', slug: 'help/contributing' },
					],
				},
			],
		}),
	],
});
