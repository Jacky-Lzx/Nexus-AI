use clipkeep_protocol::HostRequest;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct SearchContext {
    pub query_id: u64,
    cancelled: Arc<AtomicBool>,
    deadline: Option<Instant>,
}

impl SearchContext {
    pub fn new(query_id: u64, timeout_ms: Option<u64>) -> Self {
        Self {
            query_id,
            cancelled: Arc::new(AtomicBool::new(false)),
            deadline: timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms.min(300_000))),
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed) || self.deadline.is_some_and(|d| Instant::now() >= d)
    }
    pub fn check(&self) -> Result<(), String> {
        if self.is_cancelled() {
            Err("Search cancelled or deadline exceeded".into())
        } else {
            Ok(())
        }
    }
}

pub(crate) enum ReaderMessage {
    Request(HostRequest, Option<SearchContext>),
    Invalid(String),
    Closed,
}

#[derive(Default)]
struct QueueState {
    messages: VecDeque<(ReaderMessage, usize)>,
    bytes: usize,
    latest: Option<SearchContext>,
    closed: bool,
}

#[derive(Default)]
pub(crate) struct RequestQueue {
    state: Mutex<QueueState>,
    ready: Condvar,
}

impl RequestQueue {
    #[cfg(test)]
    pub fn push(&self, request: HostRequest) -> bool {
        let bytes = serde_json::to_vec(&request).unwrap().len();
        self.push_sized(request, bytes)
    }
    pub fn push_sized(&self, request: HostRequest, bytes: usize) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return false;
        }
        if let HostRequest::CancelSearch { query_id } = request {
            if let Some(context) = &state.latest
                && context.query_id == query_id
            {
                context.cancel();
            }
            for (_, context) in state.messages.iter().filter_map(|(m, _)| match m {
                ReaderMessage::Request(r, Some(c)) => Some((r, c)),
                _ => None,
            }) {
                if context.query_id == query_id {
                    context.cancel();
                }
            }
            return true;
        }
        let context = if let HostRequest::Search {
            query_id,
            timeout_ms,
            ..
        } = &request
        {
            if let Some(previous) = &state.latest {
                previous.cancel();
            }
            let context = SearchContext::new(*query_id, *timeout_ms);
            state.latest = Some(context.clone());
            // Only adjacent searches are interchangeable. Never reorder an action/configure.
            if matches!(
                state.messages.back(),
                Some((ReaderMessage::Request(HostRequest::Search { .. }, _), _))
            ) {
                let (_, removed) = state.messages.pop_back().unwrap();
                state.bytes -= removed;
            }
            Some(context)
        } else {
            None
        };
        if matches!(request, HostRequest::Shutdown)
            && let Some(context) = &state.latest
        {
            context.cancel();
        }
        if state.messages.len() >= 64 || state.bytes.saturating_add(bytes) > 32 * 1024 * 1024 {
            if let Some(context) = &state.latest {
                context.cancel();
            }
            state.messages.clear();
            state.bytes = 0;
            state.messages.push_back((
                ReaderMessage::Invalid("request queue limit exceeded".into()),
                0,
            ));
            state.closed = true;
        } else {
            state.bytes += bytes;
            state
                .messages
                .push_back((ReaderMessage::Request(request, context), bytes));
        }
        self.ready.notify_one();
        !state.closed
    }
    pub fn invalid(&self, message: String) {
        let mut state = self.state.lock().unwrap();
        if state.messages.len() < 64 {
            state
                .messages
                .push_back((ReaderMessage::Invalid(message), 0));
        }
        self.ready.notify_one();
    }
    pub fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        if let Some(context) = &state.latest {
            context.cancel();
        }
        self.ready.notify_one();
    }
    pub fn receive(&self, deadline: Option<Instant>) -> Option<ReaderMessage> {
        let mut state = self.state.lock().unwrap();
        loop {
            if deadline.is_some_and(|d| Instant::now() >= d) {
                return None;
            }
            if let Some((message, bytes)) = state.messages.pop_front() {
                state.bytes -= bytes;
                return Some(message);
            }
            if state.closed {
                return Some(ReaderMessage::Closed);
            }
            state = if let Some(deadline) = deadline {
                self.ready
                    .wait_timeout(state, deadline.saturating_duration_since(Instant::now()))
                    .unwrap()
                    .0
            } else {
                self.ready.wait(state).unwrap()
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn search(id: u64) -> HostRequest {
        HostRequest::Search {
            query_id: id,
            query: "x".into(),
            limit: 1,
            timeout_ms: None,
        }
    }
    fn id(message: Option<ReaderMessage>) -> u64 {
        match message.unwrap() {
            ReaderMessage::Request(HostRequest::Search { query_id, .. }, _) => query_id,
            _ => panic!(),
        }
    }
    #[test]
    fn typing_coalesces_without_crossing_action_barrier() {
        let q = RequestQueue::default();
        q.push(search(1));
        q.push(search(2));
        q.push(HostRequest::Execute {
            request_id: 4,
            result_id: "x".into(),
            action_id: "x".into(),
            input: None,
            arguments: None,
        });
        q.push(search(3));
        q.push(search(4));
        assert_eq!(id(q.receive(None)), 2);
        assert!(matches!(
            q.receive(None),
            Some(ReaderMessage::Request(HostRequest::Execute { .. }, _))
        ));
        assert_eq!(id(q.receive(None)), 4);
    }
    #[test]
    fn byte_limit_applies_even_below_message_count_limit() {
        let q = RequestQueue::default();
        assert!(q.push_sized(
            HostRequest::ListConfigurationItems { request_id: 1 },
            20 * 1024 * 1024
        ));
        assert!(!q.push_sized(
            HostRequest::ListConfigurationItems { request_id: 2 },
            20 * 1024 * 1024
        ));
        assert!(matches!(q.receive(None), Some(ReaderMessage::Invalid(_))));
        assert!(matches!(q.receive(None), Some(ReaderMessage::Closed)));
    }
    #[test]
    fn superseding_and_explicit_cancel_reach_running_search() {
        let q = RequestQueue::default();
        q.push(search(1));
        let Some(ReaderMessage::Request(_, Some(first))) = q.receive(None) else {
            panic!()
        };
        q.push(search(2));
        assert!(first.is_cancelled());
        let Some(ReaderMessage::Request(_, Some(second))) = q.receive(None) else {
            panic!()
        };
        q.push(HostRequest::CancelSearch { query_id: 1 });
        assert!(!second.is_cancelled());
        q.push(HostRequest::CancelSearch { query_id: 2 });
        assert!(second.is_cancelled());
        assert!(SearchContext::new(3, Some(0)).is_cancelled());
    }
}
