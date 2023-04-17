import { Env } from '.'

type Nonce = {
  id: number
  timestamp: number
}

type CheckNonceMessage = {
  message_type: 'check_nonce_is_used'
  id: number
  timestamp: number
}

type ToPeerMessage = CheckNonceMessage

export class Peer {
  state: DurableObjectState
  env: Env

  async getNonceList(): Promise<Nonce[]> {
    let nonceList: Nonce[] = (await this.state.storage.get('nonce_list')) || []
    return nonceList
  }

  constructor(state: DurableObjectState, env: Env) {
    this.state = state
    this.env = env
  }

  async handleFetch(body: ToPeerMessage): Promise<boolean> {
    switch (body.message_type) {
      case 'check_nonce_is_used': {
        body = body as CheckNonceMessage
        let nonceList = await this.getNonceList()
        if (nonceList.some(v => v.id === body.id && v.timestamp === body.timestamp)) {
          return true
        } else {
          nonceList = nonceList.filter(v => v.timestamp > Math.floor(Date.now() / 1000) - 10 * 60)
          nonceList.push({ id: body.id, timestamp: body.timestamp })
          this.state.storage.put('nonce_list', nonceList)
          this.state.storage.setAlarm(Date.now() + 60 * 11 * 1000)
          return false
        }
      }
    }
  }

  async fetch(request: Request): Promise<Response> {
    let body: ToPeerMessage = await request.json()
    let responseBody = await this.handleFetch(body)
    return new Response(JSON.stringify(responseBody))
  }

  async alarm() {
    let nonceList = await this.getNonceList()
    nonceList = nonceList.filter(v => v.timestamp > Math.floor(Date.now() / 1000) - 10 * 60)
    if (nonceList.length <= 0) {
      this.state.storage.delete('nonce_list')
    } else {
      this.state.storage.put('nonce_list', nonceList)
      this.state.storage.setAlarm(Date.now() + 60 * 11 * 1000)
    }
  }
}
