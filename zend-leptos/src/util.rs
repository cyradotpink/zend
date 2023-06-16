use std::future::Future;
use std::time::Duration;

pub async fn future_or_timeout<A>(future: A, timeout: Duration) -> Option<A::Output>
where
    A: Future + Unpin,
{
    let timeout_fut = gloo_timers::future::sleep(timeout);
    match futures::future::select(future, timeout_fut).await {
        futures::future::Either::Left((v, _)) => Some(v),
        futures::future::Either::Right(_) => None,
    }
}
