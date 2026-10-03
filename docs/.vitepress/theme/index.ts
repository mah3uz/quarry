import DefaultTheme from 'vitepress/theme'
import type { Theme } from 'vitepress'
import { init } from '@plausible-analytics/tracker'
import '@fontsource-variable/recursive/full.css'
import './style.css'
import Terminal from './components/Terminal.vue'
import HomePage from './components/HomePage.vue'

export default {
  extends: DefaultTheme,
  enhanceApp({ app }) {
    app.component('Terminal', Terminal)
    app.component('HomePage', HomePage)
    if (!import.meta.env.SSR) {
      init({ domain: 'quarry.asmechanics.com', endpoint: 'https://plus.testtlc.com/api/event' })
    }
  },
} satisfies Theme
