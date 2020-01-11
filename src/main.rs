use core::task::Poll;
use core::task::{Context, Waker};
use core::future::Future;
use core::pin::Pin;

#[derive(PartialEq, PartialOrd, Eq, Ord, Clone)]
struct TaskId(pub u64);

// 最初のタスクの定義。標準は使わない。
struct Task<T> {
     fut: Pin<Box<dyn Future<Output = T>>>,
}

impl<T> Task<T> {
    fn new(f: impl Future<Output = T> + 'static) -> Self {
        Self {
            fut: Box::pin(f)
        }
    }

    // Wakerを使ってfutureを1ステップ進めるだけ
    fn run_step(&mut self, waker: &Waker) -> Poll<T> {
        let f = self.fut.as_mut();
        let mut ctx = Context::from_waker(waker);
        Future::poll(f, &mut ctx)
    }
}

////

use std::sync::Arc;
use std::mem::ManuallyDrop;
use std::task::{RawWaker, RawWakerVTable};

static VTABLE: RawWakerVTable = RawWakerVTable::new(
    raw_clone,
    raw_wake,
    raw_wake_by_ref,
    raw_drop
);

struct WakerInner {
}

impl WakerInner {
    fn new() -> Self {
        Self {}
    }

    fn wake(&self) {
    }
}

fn raw_clone(ptr: *const ()) -> RawWaker {
    // has NOT ownership
    let i = ManuallyDrop::new(unsafe { Arc::from_raw(ptr as *const WakerInner) });
    std::mem::forget(i.clone()); // clone and do NOT drop (increment ref-counter of ptr)
    RawWaker::new(ptr, &VTABLE)
}

fn raw_wake(ptr: *const ()) {
    // has ownership of waker
    let i = unsafe { Arc::from_raw(ptr as *const WakerInner) }; // Droped by Arc
    i.wake()
}

fn raw_wake_by_ref(ptr: *const ()) {
    // has NOT ownership, keep ref-counter (will NOT decremented)
    let i = ManuallyDrop::new(unsafe { Arc::from_raw(ptr as *const WakerInner) });
    i.wake()
}

fn raw_drop(ptr: *const ()) {
    unsafe { Arc::from_raw(ptr as *const WakerInner) }; // Droped by Arc
}

fn new_waker() -> Waker {
    let inner = Arc::new(WakerInner::new());
    unsafe { Waker::from_raw(raw_clone(Arc::into_raw(inner) as *const ())) }
}

////

use std::collections::VecDeque;
use std::collections::BTreeMap;

struct Executor {
    run_queue: VecDeque<TaskId>,
    task_pool: BTreeMap<TaskId, Task<()>>,
    task_id: u64,
}

impl Executor {
    fn new() -> Self {
        Self {
            run_queue: VecDeque::new(),
            task_pool: BTreeMap::new(),
            task_id: 0,
        }
    }

    fn fresh_id(&mut self) -> TaskId {
        let tid = self.task_id;
        self.task_id += 1;

        TaskId(tid)
    }

    fn spawn(&mut self, f: impl Future<Output = ()> + 'static) {
        let task = Task::new(f);
        let task_id = self.fresh_id();
        self.task_pool.insert(task_id.clone(), task);

        self.run_queue.push_back(task_id)
    }

    fn run(&mut self) {
        loop {
            if self.task_pool.is_empty() {
                break;
            }

            while let Some(task_id) = self.run_queue.pop_front() {
                let polled = {
                    let task = self.task_pool.get_mut(&task_id)
                        .expect("task_id must exists");

                    let waker = new_waker();
                    task.run_step(&waker)
                };
                match polled {
                    Poll::Ready(_) => {
                        let _ = self.task_pool.remove(&task_id);
                    },
                    Poll::Pending => {
                    }
                }
            }
        }
    }
}

////

fn main() {
    let mut executor = Executor::new();

    executor.spawn(f(1));
    executor.spawn(f(2));

    executor.run()
}

async fn f(i: usize) {
    println!("Hello, world! : {}", i);
}
