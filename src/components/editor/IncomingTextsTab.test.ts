// @vitest-environment jsdom
import { createApp, h, nextTick } from 'vue'
import { afterEach, describe, expect, it } from 'vitest'
import type { JobDto, JobStatus } from '../../../src-playback/speechQueue'
import IncomingTextsTab from './IncomingTextsTab.vue'

const apps: ReturnType<typeof createApp>[] = []
afterEach(() => {
  for (const app of apps.splice(0)) app.unmount()
  document.body.innerHTML = ''
})

function externalJob(status: JobStatus, id = `job-${status}`): JobDto {
  return {
    job_id: id,
    original_text: `text for ${status}`,
    spoken_text: null,
    status,
    error: null,
    attempt: 1,
    created_at_ms: 0,
    last_activity_at_ms: 0,
    source: 'server',
  }
}

interface MountOptions {
  jobs: JobDto[]
  editDisabled?: boolean
  busyIds?: ReadonlySet<string>
}

function mountTab(options: MountOptions) {
  const emitted = { edits: [] as string[], cancels: [] as string[] }
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({
    setup: () => () =>
      h(IncomingTextsTab, {
        pendingItems: [],
        externalJobs: options.jobs,
        autoPlay: true,
        route: 'audio_only',
        twitchConnected: false,
        webviewConnected: false,
        vrchatEnabled: false,
        busyIds: options.busyIds ?? new Set<string>(),
        editDisabled: options.editDisabled ?? false,
        compact: false,
        loadError: null,
        onEditExternal: (jobId: string) => { emitted.edits.push(jobId) },
        onCancelExternal: (jobId: string) => { emitted.cancels.push(jobId) },
      }),
  })
  apps.push(app)
  app.mount(host)
  return { host, emitted }
}

function button(host: HTMLElement, selector: string): HTMLButtonElement {
  const found = host.querySelector<HTMLButtonElement>(selector)
  expect(found).not.toBeNull()
  return found as HTMLButtonElement
}

describe('IncomingTextsTab external jobs', () => {
  it('edits and cancels a failed external job', async () => {
    const { host, emitted } = mountTab({ jobs: [externalJob('failed', 'job-1')] })

    const edit = button(host, '.incoming-btn.edit')
    const discard = button(host, '.incoming-btn.discard')
    expect(edit.disabled).toBe(false)
    expect(discard.disabled).toBe(false)

    edit.click()
    await nextTick()
    expect(emitted.edits).toEqual(['job-1'])

    discard.click()
    await nextTick()
    expect(emitted.cancels).toEqual(['job-1'])
  })

  it.each<JobStatus>(['queued', 'generating', 'ready'])(
    'only cancels a %s external job',
    (status) => {
      const { host } = mountTab({ jobs: [externalJob(status, `job-${status}`)] })

      expect(host.querySelector('.incoming-btn.edit')).toBeNull()
      const discard = button(host, '.incoming-btn.discard')
      expect(discard.title.length).toBeGreaterThan(0)
    },
  )

  it('shows no actions for a playing external job', () => {
    const { host } = mountTab({ jobs: [externalJob('playing', 'job-playing')] })

    expect(host.querySelector('.incoming-actions')).toBeNull()
    expect(host.querySelector('.incoming-btn')).toBeNull()
  })

  it('disables only edit while another take is in flight', () => {
    const { host } = mountTab({
      jobs: [externalJob('failed', 'job-1')],
      editDisabled: true,
    })

    expect(button(host, '.incoming-btn.edit').disabled).toBe(true)
    expect(button(host, '.incoming-btn.discard').disabled).toBe(false)
  })
})
