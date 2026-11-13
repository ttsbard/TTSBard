import { createApp } from 'vue'
import SoundPanelApp from './SoundPanelApp.vue'
import { bootstrapLocalization, i18n } from '../src/i18n'

async function start(): Promise<void> {
  await bootstrapLocalization()
  const app = createApp(SoundPanelApp)
  app.use(i18n)
  app.mount('#app')
}

void start()
