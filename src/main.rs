use core::future::Future;
use core::pin::Pin;
use core::task::Poll;
use core::task::{Context, Waker};
use std::sync::Arc;

#[derive(PartialEq, PartialOrd, Eq, Ord, Clone)]
struct TaskId(pub u64);

// 最初のタスクの定義。標準は使わない。
struct Task<T> {
    fut: Pin<Box<dyn Future<Output = T>>>,
}

type TaskRef<T> = Arc<RefCell<Task<T>>>;

impl<T> Task<T> {
    fn new(f: impl Future<Output = T> + 'static) -> TaskRef<T> {
        Arc::new(RefCell::new(Self { fut: Box::pin(f) }))
    }

    // Wakerを使ってfutureを1ステップ進めるだけ
    fn run_step(&mut self, waker: &Waker) -> Poll<T> {
        let f = self.fut.as_mut();
        let mut ctx = Context::from_waker(waker);
        Future::poll(f, &mut ctx)
    }
}

////

use core::mem::ManuallyDrop;
use core::task::{RawWaker, RawWakerVTable};

static VTABLE: RawWakerVTable =
    { RawWakerVTable::new(raw_clone, raw_wake, raw_wake_by_ref, raw_drop) };

struct WakerInner {
    task: TaskRef<()>,
}

impl WakerInner {
    fn new(task: TaskRef<()>) -> Self {
        Self { task }
    }

    fn wake(&self) {
        // 今回の実装では、仮にthread_localのExecutorを見つけてきてそこにスケジューリングする
        EXECUTOR.with(|e| e.borrow().schedule(self.task.clone()))
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

fn new_waker(task: TaskRef<()>) -> Waker {
    let inner = Arc::new(WakerInner::new(task));
    unsafe { Waker::from_raw(raw_clone(Arc::into_raw(inner) as *const ())) }
}

////

use std::collections::VecDeque;

struct TaskQueue {
    run_queue: RefCell<VecDeque<TaskRef<()>>>,
}

impl TaskQueue {
    fn new() -> Self {
        Self {
            run_queue: Default::default(),
        }
    }

    fn enqueue(&self, t: TaskRef<()>) {
        self.run_queue.borrow_mut().push_back(t)
    }

    fn dequeue(&self) -> Option<TaskRef<()>> {
        self.run_queue.borrow_mut().pop_front()
    }
}

struct Executor {
    queue: TaskQueue,
}

impl Executor {
    fn new() -> Self {
        Self {
            queue: TaskQueue::new(),
        }
    }

    fn spawn(&self, f: impl Future<Output = ()> + 'static) {
        let task = Task::new(f);

        self.schedule(task);
    }

    fn schedule(&self, task: TaskRef<()>) {
        self.queue.enqueue(task);
    }

    fn take_scheduled(&self) -> Option<TaskRef<()>> {
        self.queue.dequeue()
    }

    fn run(&self) {
        while let Some(task) = self.take_scheduled() {
            let waker = new_waker(task.clone());
            let _ = task.borrow_mut().run_step(&waker);
        }
    }
}

////

// 動作確認用のテストFuture。
// 1度目のpollは中断。2回目で完了にしてくれる。
struct OneSkipFuture<T>
where
    T: std::marker::Unpin + Clone,
{
    t: T,
    twice: bool,
}

impl<T> Future for OneSkipFuture<T>
where
    T: std::marker::Unpin + Clone,
{
    type Output = T;

    fn poll(self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {
        let f = self.get_mut();
        if f.twice {
            // 2回目なので完了
            Poll::Ready(f.t.clone())
        } else {
            // 1回目のpollはこちらのフロー
            f.twice = true; // 次は完了にするために状態を書き換え

            // もう一度スケジューリングしてもらうために、すぐさまwakeしておく。
            // ctxからはアドレスしか取れないので、wake_by_refを使う
            let waker = ctx.waker();
            waker.wake_by_ref();

            // 一旦中断
            Poll::Pending
        }
    }
}

//// 現状スレッド1つにExecutor1つ

use core::cell::RefCell;

thread_local! {
    static EXECUTOR: RefCell<Executor> = RefCell::new(Executor::new());
}

fn spawn(f: impl Future<Output = ()> + 'static) {
    EXECUTOR.with(|e| {
        e.borrow().spawn(f);
    })
}

fn run() {
    EXECUTOR.with(|e| {
        e.borrow().run();
    })
}

////

fn main() {
    spawn(f(1));
    spawn(f(2));

    // 以下のように出力される
    //
    // f-before : 1
    // f-before : 2
    // f-end : 1
    // f-end : 2
    //
    // "f-end : 1" が呼ばれる前に処理が中断して、先に"f-before : 2"が呼ばれている
    run()
}

async fn f(i: usize) {
    println!("f-before : {}", i);
    one_skip(i).await; // ここで一旦pendingになり、スケジュールが他のタスクに移るはず
    println!("f-end : {}", i);
}

fn one_skip(i: usize) -> OneSkipFuture<usize> {
    OneSkipFuture { t: i, twice: false }
}
