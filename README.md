# zend (WIP)
Short-lived encrypted sessions

Zend will be a tiny web app enabling quick, uncomplicated exchange of encrypted messages between devices.\
A major use case for me will be easy distribution of passwords to people or devices that no more permanent secure channel exists to yet.

## This repository contains:

### A work-in-progress cloudflare worker
... that will act as the central API. It plays no role in the security of the peer-to-peer message encryption,
but makes an effort to pre-validate as much as it can and attempts to ensure the
reliability and availability of the service.\
State is managed using Cloudflare's Durable Objects,
so there's pretty major vendor lock-in there, but it's neat tech that I wanted play around with. Sadly Workers' rust bindings
are currently very incomplete when it comes to Durable Objects, so I had to write that part in Typescript, which is annoying.\
Right now, The worker compiles and runs, and responds to websocket messages, but is not tested well and some functionality is
unimplemented.

### A work-in-progress web app
... intended to function as the service's main client. Since this whole project is really just an excuse to get some Rust experience
(and because I want to share API data structures between server and client), I'll be trying to write this part, too, in Rust,
despite Rust frontend frameworks still seeming very experimental.
