use futures::{Async, Future, Stream, Poll, IntoFuture};

pub struct ForEach<I>(I);

pub fn for_each<J, T>(i: J) -> ForEach<J::IntoIter>
    where J: IntoIterator<Item=T>
{
    ForEach(i.into_iter())
}


impl<I, T> Stream for ForEach<I>
    where I: Iterator<Item=T>
{
    type Item = T;
    type Error = ();

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self.0.next() {
            Some(v) => Ok(Async::Ready(Some(v))),
            None => Ok(Async::Ready(None))
        }
    }
}

impl<T> Until for ForEach<T> {}


pub trait Until {

    fn until<P, F, R, E>(self, p: P) -> UntilFut<Self, P, F, R, E>
        where P: FnMut(&Self::Item) -> F,
              F: IntoFuture<Item=Option<R>, Error=E>,
              Self: Stream + Sized,
    {
        UntilFut::new(self, p)
    }
}


pub enum UntilError<S, F> where S: Stream, F: Future {
    NoResult,
    Error(F::Error),
    StreamError(S::Error),
}


pub struct UntilFut<S, P, F, R, E> where S: Stream, F: IntoFuture<Item=Option<R>, Error=E> {
    stream: S,
    pred: P,
    pending: Option<F::Future>,
    done: bool,
}

impl<S, P, F, R, E> UntilFut<S, P, F, R, E>
    where S: Stream,
          P: FnMut(&S::Item) -> F,
          F: IntoFuture<Item=Option<R>, Error=E>,
{
    pub fn new(s: S, p: P) -> UntilFut<S, P, F, R, E>
    {
        UntilFut {
            stream: s,
            pred: p,
            pending: None,
            done: false,
        }
    }
}

impl<S, P, F, R, E> Future for UntilFut<S, P, F, R, E>
    where S: Stream,
          P: FnMut(&S::Item) -> F,
          F: IntoFuture<Item=Option<R>, Error=E>,
{
    type Item = R;
    type Error = UntilError<S, F::Future>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if self.done {
            panic!()  // can not poll completed future
        }

        if self.pending.is_none() {
            let item = match self.stream.poll() {
                Ok(Async::Ready(Some(item))) => item,
                Ok(Async::Ready(None)) => return Err(UntilError::NoResult),
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Err(e) => return Err(UntilError::StreamError(e)),
            };
            self.pending = Some((self.pred)(&item).into_future());
        }

        assert!(self.pending.is_some());
        match self.pending.as_mut().unwrap().poll() {
            Ok(Async::Ready(None)) => {
                self.pending = None;
                self.poll()
            },
            Ok(Async::Ready(Some(result))) => {
                self.done = true;
                Ok(Async::Ready(result))
            }
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(e) => {
                self.pending = None;
                Err(UntilError::Error(e))
            }
        }
    }
}
