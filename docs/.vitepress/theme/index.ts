import DefaultTheme from 'vitepress/theme'
import type { Theme } from 'vitepress'
import '@fontsource-variable/recursive/full.css'
import './style.css'
import Terminal from './components/Terminal.vue'
import HomePage from './components/HomePage.vue'

export default {
  extends: DefaultTheme,
  enhanceApp({ app }) {
    app.component('Terminal', Terminal)
    app.component('HomePage', HomePage)
  },
} satisfies Theme
