import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { encode, type Event, type RunAgentInput } from '@rusty-mill/agui-core'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { renderApp } from '@/test/renderApp'
import { useUi } from '@/store/ui'

/**
 * The real panel, store and hooks over a scripted agent that answers the way
 * `rusty_tick`'s assistant does: a tool call to add the task, then a
 * confirmation once the UI has answered it. Only `/api/agent` is faked.
 */
function scriptedAgent(): { fetch: typeof fetch; inputs: RunAgentInput[] } {
  const inputs: RunAgentInput[] = []
  const say = (id: string, text: string): Event[] => [
    { type: 'TEXT_MESSAGE_START', messageId: id, role: 'assistant' },
    { type: 'TEXT_MESSAGE_CONTENT', messageId: id, delta: text },
    { type: 'TEXT_MESSAGE_END', messageId: id },
  ]
  const fetchImpl: typeof fetch = async (url, init) => {
    expect(String(url)).toBe('/api/agent')
    const input = JSON.parse(String(init?.body)) as RunAgentInput
    inputs.push(input)
    const last = input.messages?.at(-1)
    const { threadId, runId } = input
    let inner: Event[]
    if (last?.role === 'tool') {
      const { title } = JSON.parse(String(last.content)) as { title: string }
      inner = say('a2', `Added “${title}”.`)
    } else {
      inner = [
        ...say('a1', 'Adding “Buy milk”…'),
        { type: 'TOOL_CALL_START', toolCallId: 'c1', toolCallName: 'create_task', parentMessageId: 'a1' },
        { type: 'TOOL_CALL_ARGS', toolCallId: 'c1', delta: JSON.stringify({ title: 'Buy milk' }) },
        { type: 'TOOL_CALL_END', toolCallId: 'c1' },
      ]
    }
    const body = [{ type: 'RUN_STARTED', threadId, runId } as Event, ...inner, { type: 'RUN_FINISHED', threadId, runId } as Event].map(encode).join('')
    return new Response(new TextEncoder().encode(body), { status: 200, headers: { 'content-type': 'text/event-stream' } })
  }
  return { fetch: fetchImpl, inputs }
}

afterEach(() => {
  vi.unstubAllGlobals()
  useUi.getState().closeAssistant()
})

describe('the assistant panel', () => {
  it('opens from the rail, adds a task through the create_task tool, and confirms', async () => {
    const agent = scriptedAgent()
    vi.stubGlobal('fetch', agent.fetch)
    const user = userEvent.setup()
    const { api } = await renderApp('/p/inbox/tasks')

    await user.click(screen.getByRole('button', { name: 'Assistant' }))
    const panel = within(screen.getByRole('complementary', { name: 'Assistant' }))
    await user.type(panel.getByLabelText('Message the assistant'), 'add Buy milk{Enter}')

    await waitFor(() => expect(panel.getByText('Added “Buy milk”.')).toBeInTheDocument())
    const tasks = (await api.snapshot()).tasks
    expect(tasks.map((t) => t.title)).toEqual(['Buy milk'])

    // The first run offered the tool and the open view; the second carried the tool's answer.
    expect(agent.inputs).toHaveLength(2)
    expect(agent.inputs[0]?.tools?.map((t) => t.name)).toEqual(['create_task'])
    expect(agent.inputs[0]?.context?.map((c) => c.description)).toEqual(['view', 'tasks in view'])
    expect(agent.inputs[1]?.messages?.at(-1)).toMatchObject({ role: 'tool', toolCallId: 'c1' })
    expect(JSON.parse(String(agent.inputs[1]?.messages?.at(-1)?.content))).toMatchObject({ title: 'Buy milk' })

    await user.click(panel.getByRole('button', { name: 'Close assistant' }))
    expect(screen.queryByRole('complementary', { name: 'Assistant' })).not.toBeInTheDocument()
  })

  it('shows a transport failure and stays usable', async () => {
    vi.stubGlobal('fetch', async () => new Response('nope', { status: 404 }))
    const user = userEvent.setup()
    await renderApp('/p/inbox/tasks')
    await user.click(screen.getByRole('button', { name: 'Assistant' }))
    const panel = within(screen.getByRole('complementary', { name: 'Assistant' }))
    await user.type(panel.getByLabelText('Message the assistant'), 'help{Enter}')
    await waitFor(() => expect(panel.getByRole('alert')).toHaveTextContent(/404/))
    expect(panel.getByLabelText('Message the assistant')).toBeEnabled()
  })
})
