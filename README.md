# zend (WIP)
Short-lived encrypted sessions

Zend will be a tiny web app enabling quick, uncomplicated exchange of encrypted messages between devices.\
A major use case for me will be easy distribution of passwords to people or devices that no more permanent secure channel exists to yet.

## This repository contains:

### A work-in-progress cloudflare worker
... that will act as the central API. It plays no role in the security of the peer-to-peer message encryption,
but makes an effort to pre-validate as much as it can and attempts to ensure the
reliability and availability of the service. It also tries its best to hide the existence of encryption sessions from clients
who are not intended by the sessions' legitimate peers to take part in the communication.
A server-side compromise of the application would expose the presence of sessions, peers, and messages, but not the encrypted communication
between session peers.

### A work-in-progress web app
... intended to function as the service's main client.