import { config } from '@vue/test-utils'

class ResizeObserverMock {
  observe() {}
  unobserve() {}
  disconnect() {}
}

vi.stubGlobal('ResizeObserver', ResizeObserverMock)
vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 0 })
vi.stubGlobal('cancelAnimationFrame', () => {})
config.global.stubs = { Teleport: true }
