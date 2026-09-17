import { mount } from '@vue/test-utils'
import McpServerList from '~/components/McpServerList.vue'
import { describeMcpServer, parseMcpServers } from '~/lib/mcpServers'

/**
 * A configured MCP server that failed to start used to be invisible in the
 * GUI — its tools were just missing. The daemon now reports each server's
 * state; these pin how the Tools page reads and words it.
 */
describe('parseMcpServers', () => {
  const daemon = [
    { name: 'fixture', state: 'started', tools: 1 },
    { name: 'broken', state: 'failed', tools: 0, detail: 'Failed to spawn server' },
    { name: '', state: 'not_started', tools: 0, detail: "MCP server name 'fixture' is used twice" },
  ]

  it('reads the daemon wire shape (captured from a live daemon)', () => {
    expect(parseMcpServers(daemon)).toEqual(daemon)
  })

  it('drops malformed entries and never throws', () => {
    expect(parseMcpServers(null)).toEqual([])
    expect(parseMcpServers({ servers: daemon })).toEqual([])
    expect(
      parseMcpServers([
        null,
        { name: 'x', state: 'exploded', tools: 0 },
        { name: 'x', state: 'started', tools: -1 },
        { name: 'x', state: 'started', tools: 1.5 },
        { name: '', state: 'not_started', tools: 0 },
        { name: 'ok', state: 'started', tools: 2 },
      ]),
    ).toEqual([{ name: 'ok', state: 'started', tools: 2 }])
  })
})

describe('describeMcpServer', () => {
  it('says what each state means', () => {
    expect(describeMcpServer({ name: 'a', state: 'started', tools: 1 })).toBe('1 tool')
    expect(describeMcpServer({ name: 'a', state: 'started', tools: 3 })).toBe('3 tools')
    expect(describeMcpServer({ name: 'a', state: 'starting', tools: 0 })).toBe('starting…')
    expect(describeMcpServer({ name: 'a', state: 'failed', tools: 0, detail: 'ENOENT' })).toBe('failed: ENOENT')
    expect(describeMcpServer({ name: '', state: 'not_started', tools: 0, detail: 'dup' })).toBe('dup')
  })
})

describe('McpServerList', () => {
  it('renders nothing when no server is configured', () => {
    const wrapper = mount(McpServerList, { props: { servers: [] } })
    expect(wrapper.find('section').exists()).toBe(false)
  })

  it('renders one row per server with its state and reason', () => {
    const wrapper = mount(McpServerList, {
      props: { servers: parseMcpServers([
        { name: 'fixture', state: 'started', tools: 1 },
        { name: 'broken', state: 'failed', tools: 0, detail: 'ENOENT' },
      ]) },
    })
    const rows = wrapper.findAll('li')
    expect(rows).toHaveLength(2)
    expect(rows[0]!.attributes('data-state')).toBe('started')
    expect(rows[0]!.text()).toContain('fixture')
    expect(rows[1]!.text()).toContain('failed: ENOENT')
  })
})
