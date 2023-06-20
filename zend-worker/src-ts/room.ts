import { Env } from '.'

function timestampFromNonce(nonce: string) {
  return parseInt(nonce.split('_')[1])
}

type InitialiseMessage = {
  message_type: 'initialise'
  initial_peer_id: string
}

type CheckExistsMessage = {
  message_type: 'check_exists'
}

type SubscribeMessage = {
  message_type: 'subscribe'
  subscriber_id: string
}

type UnsubscribeMessage = {
  message_type: 'unsubscribe'
  subscription_id: number
}

type AddPrivilegedPeerMessage = {
  message_type: 'add_privileged_peer'
  added_id: string
  adder_id: string
}

type DeleteMessage = {
  message_type: 'delete'
  deleter_id: string | null
}

type BroadcastDataMessage = {
  message_type: 'broadcast_data'
  data: any
  sender_id: string
  nonce: string
  write_history: boolean
}

type UnicastDataMessage = {
  message_type: 'unicast_data'
  data: any
  sender_id: string
  nonce: string
  receiver_id: string
  write_history: boolean
  make_receiver_privileged: boolean
}

type DeleteDataMessage = {
  message_type: 'delete_data'
  deleter_id: string
  data_sender_id: string
  data_nonce: string
}

type HistoryEntry = {
  receiver_id: string | null
  timestamp: number
  data: any
  sender_id: string
  nonce: string
}

type Subscription = {
  socket: WebSocket
  subscriber_id: string
  subscription_id: number
}

type ToRoomMessage =
  | InitialiseMessage
  | CheckExistsMessage
  | SubscribeMessage
  | UnsubscribeMessage
  | AddPrivilegedPeerMessage
  | DeleteMessage
  | BroadcastDataMessage
  | UnicastDataMessage
  | DeleteDataMessage

export class Room {
  state: DurableObjectState
  env: Env
  subscriptions: Subscription[] = []

  constructor(state: DurableObjectState, env: Env) {
    this.state = state
    this.env = env
  }

  async getPrivilegedPeers(): Promise<string[]> {
    let result: string[] | undefined = await this.state.storage.get('privileged_peers')
    if (result === undefined) result = []
    return result
  }

  async getNextSubId(): Promise<number> {
    let next: number | undefined = await this.state.storage.get('subscription_id')
    // Randomly initialise subscription ID and restrict to same range as initial choice
    // to avoid leaking information about the existence of rooms
    if (next === undefined) {
      next = crypto.getRandomValues(new Uint32Array(1))[0] // cryptographically random 32 bit uint
    }
    this.state.storage.put('subscription_id', (next + 1) % 2 ** 32)
    return next
  }

  async exists(): Promise<boolean> {
    return (await this.getPrivilegedPeers()).length > 0
  }

  async keepAlive(peerId: string) {
    if (!(await this.getPrivilegedPeers()).includes(peerId)) return
    this.state.storage.setAlarm(Date.now() + 20 * 60 * 1000)
  }

  async addPrivilegedPeer(adder_id: string, added_id: string) {
    let peers = await this.getPrivilegedPeers()
    if (!peers.includes(adder_id)) return false
    if (!peers.includes(added_id)) {
      peers.push(added_id)
      this.state.storage.put('privileged_peers', peers)
    }
    return true
  }

  async handleFetch(body: ToRoomMessage): Promise<null | boolean | [number, WebSocket | null]> {
    switch (body.message_type) {
      case 'check_exists': {
        return await this.exists()
      }
      case 'initialise': {
        body = body as InitialiseMessage
        if (await this.exists()) {
          return false
        }
        this.state.storage.put('privileged_peers', [body.initial_peer_id])
        this.state.storage.put('message_history', [])
        this.keepAlive(body.initial_peer_id)
        return true
      }
      case 'subscribe': {
        let subscription_id = await this.getNextSubId()
        if (!(await this.exists())) return [subscription_id, null]
        body = body as SubscribeMessage
        let pair = new WebSocketPair()
        let client = pair[0]
        let server = pair[1]
        server.addEventListener('close', _ => {
          this.subscriptions = this.subscriptions.filter(v => v.socket !== server)
        })
        this.subscriptions.push({
          socket: server,
          subscriber_id: body.subscriber_id,
          subscription_id
        })
        client.send(
          JSON.stringify({ message_type: 'subscription_id', message_content: subscription_id })
        )
        return [subscription_id, client]
      }
      case 'unsubscribe': {
        if (!(await this.exists())) return null
        body = body as UnsubscribeMessage
        let subscription_id = body.subscription_id

        for (let { socket } of this.subscriptions.filter(
          v => v.subscription_id == subscription_id
        )) {
          socket.send(JSON.stringify({ message_type: 'close' }))
        }
        return null
      }
      case 'add_privileged_peer': {
        body = body as AddPrivilegedPeerMessage
        return this.addPrivilegedPeer(body.adder_id, body.added_id)
      }
      case 'delete': {
        body = body as DeleteMessage
        if (
          body.deleter_id !== null &&
          !(await this.getPrivilegedPeers()).includes(body.deleter_id)
        )
          return false
        this.state.storage.delete(['privileged_peers', 'message_history'])
        this.state.storage.deleteAlarm()
        return true
      }
      case 'broadcast_data': {
        if (!(await this.exists())) return false
        body = body as BroadcastDataMessage
        let result = await this.state.storage.get(['message_history', 'privileged_peers'])
        let privileged_peers = (result.get('privileged_peers') as string[] | undefined) || []
        if (body.write_history) {
          let history = (result.get('message_history') as HistoryEntry[] | undefined) || []
          history.push({
            receiver_id: null,
            timestamp: timestampFromNonce(body.nonce),
            data: body.data,
            sender_id: body.sender_id,
            nonce: body.nonce
          })
          this.state.storage.put('message_history', history)
        }
        for (let sub of this.subscriptions.filter(sub =>
          privileged_peers.includes(sub.subscriber_id)
        )) {
          sub.socket.send(
            JSON.stringify({
              message_type: 'data',
              message_content: { data: body.data, sender_id: body.sender_id, nonce: body.nonce }
            })
          )
        }
        this.keepAlive(body.sender_id)
        return true
      }
      case 'unicast_data': {
        if (!(await this.exists())) return false
        body = body as UnicastDataMessage
        if (body.make_receiver_privileged) {
          this.addPrivilegedPeer(body.sender_id, body.receiver_id)
        }
        if (body.write_history) {
          let history =
            ((await this.state.storage.get('message_history')) as HistoryEntry[] | undefined) || []
          history.push({
            receiver_id: body.receiver_id,
            timestamp: timestampFromNonce(body.nonce),
            data: body.data,
            sender_id: body.sender_id,
            nonce: body.nonce
          })
          this.state.storage.put('message_history', history)
        }
        let id = body.receiver_id
        for (let sub of this.subscriptions.filter(sub => id == sub.subscriber_id)) {
          sub.socket.send(
            JSON.stringify({
              message_type: 'data',
              message_content: { data: body.data, sender_id: body.sender_id, nonce: body.nonce }
            })
          )
        }
        this.keepAlive(body.sender_id)
        return true
      }
      case 'delete_data': {
        body = body as DeleteDataMessage
        let result = await this.state.storage.get(['message_history', 'privileged_peers'])
        let peers = (result.get('privileged_peers') as string[] | undefined) || []
        let deleter_id = body.deleter_id
        if (!peers.some(v => v == deleter_id)) {
          return false
        }
        let history = (result.get('message_history') as HistoryEntry[] | undefined) || []
        let nonce = body.data_nonce
        let sender_id = body.data_sender_id
        history = history.filter(v => v.nonce !== nonce || v.sender_id !== sender_id)
        this.state.storage.put('message_history', history)
        return true
      }
    }
  }

  async fetch(request: Request): Promise<Response> {
    let body: ToRoomMessage = await request.json()
    let responseBody = await this.handleFetch(body)
    if (responseBody instanceof Array) {
      let [id, ws] = responseBody
      // return new Response(JSON.stringify(null))
      return new Response(ws ? null : '', {
        status: ws ? 101 : 200,
        webSocket: ws,
        headers: { 'Subscription-Id': id.toString() }
      })
    } else {
      return new Response(JSON.stringify(responseBody))
    }
  }

  async alarm() {
    this.state.storage.delete(['privileged_peers', 'message_history'])
  }
}
