use crate::float::FftFloat;
use safe_simd::vector::{Complex, Loader, VectorCore};

pub(crate) trait Butterfly<T: FftFloat, V: Complex<T>> {
    type Buffer: AsRef<[V]> + AsMut<[V]>;

    fn radix() -> usize;

    fn make_buffer<L: Loader<V::Scalar, Vector = V>>(handle: L) -> Self::Buffer;

    fn apply<L: Loader<V::Scalar, Vector = V>>(
        handle: L,
        input: Self::Buffer,
        forward: bool,
    ) -> Self::Buffer;
}

pub(crate) struct Butterfly2;

impl<T: FftFloat, V: Complex<T>> Butterfly<T, V> for Butterfly2 {
    type Buffer = [V; 2];

    #[inline(always)]
    fn radix() -> usize {
        2
    }

    #[inline(always)]
    fn make_buffer<L: Loader<V::Scalar, Vector = V>>(handle: L) -> Self::Buffer {
        [handle.zeroed(), handle.zeroed()]
    }

    #[inline(always)]
    fn apply<L: Loader<V::Scalar, Vector = V>>(
        _handle: L,
        input: Self::Buffer,
        _forward: bool,
    ) -> Self::Buffer {
        [input[0] + input[1], input[0] - input[1]]
    }
}

pub(crate) struct Butterfly3;

impl<T: FftFloat, V: Complex<T>> Butterfly<T, V> for Butterfly3 {
    type Buffer = [V; 3];

    #[inline(always)]
    fn radix() -> usize {
        3
    }

    #[inline(always)]
    fn make_buffer<L: Loader<V::Scalar, Vector = V>>(handle: L) -> Self::Buffer {
        [handle.zeroed(), handle.zeroed(), handle.zeroed()]
    }

    #[inline(always)]
    fn apply<L: Loader<V::Scalar, Vector = V>>(
        handle: L,
        input: Self::Buffer,
        forward: bool,
    ) -> Self::Buffer {
        let t = crate::twiddle::compute_twiddle(1, 3, forward);
        let twiddle = handle.splat(t);
        let twiddle_conj = handle.splat(t);
        [
            input[0] + input[1] + input[2],
            input[0] + input[1] * twiddle + input[2] * twiddle_conj,
            input[0] + input[1] * twiddle_conj + input[2] * twiddle,
        ]
    }
}

pub(crate) struct Butterfly4;

impl<T: FftFloat, V: Complex<T>> Butterfly<T, V> for Butterfly4 {
    type Buffer = [V; 4];

    #[inline(always)]
    fn radix() -> usize {
        4
    }

    #[inline(always)]
    fn make_buffer<L: Loader<V::Scalar, Vector = V>>(handle: L) -> Self::Buffer {
        [
            handle.zeroed(),
            handle.zeroed(),
            handle.zeroed(),
            handle.zeroed(),
        ]
    }

    #[inline(always)]
    fn apply<L: Loader<V::Scalar, Vector = V>>(
        handle: L,
        input: Self::Buffer,
        forward: bool,
    ) -> Self::Buffer {
        let mut a = {
            let a0 = Butterfly2::apply(handle, [input[0], input[2]], forward);
            let a1 = Butterfly2::apply(handle, [input[1], input[3]], forward);
            [a0[0], a0[1], a1[0], a1[1]]
        };
        a[3] = if forward {
            a[3].mul_i()
        } else {
            a[3].mul_neg_i()
        };
        let b = {
            let b0 = Butterfly2::apply(handle, [a[0], a[2]], forward);
            let b1 = Butterfly2::apply(handle, [a[1], a[3]], forward);
            [b0[0], b0[1], b1[0], b1[1]]
        };
        [b[0], b[3], b[1], b[2]]
    }
}

pub(crate) struct Butterfly8;

impl<T: FftFloat, V: Complex<T>> Butterfly<T, V> for Butterfly8 {
    type Buffer = [V; 8];

    #[inline(always)]
    fn radix() -> usize {
        8
    }

    #[inline(always)]
    fn make_buffer<L: Loader<V::Scalar, Vector = V>>(handle: L) -> Self::Buffer {
        [
            handle.zeroed(),
            handle.zeroed(),
            handle.zeroed(),
            handle.zeroed(),
            handle.zeroed(),
            handle.zeroed(),
            handle.zeroed(),
            handle.zeroed(),
        ]
    }

    #[inline(always)]
    fn apply<L: Loader<V::Scalar, Vector = V>>(
        handle: L,
        input: Self::Buffer,
        forward: bool,
    ) -> Self::Buffer {
        let t = crate::twiddle::compute_twiddle(1, 8, forward);
        let twiddle = handle.splat(t);
        let twiddle_neg = handle.splat(num_complex::Complex::new(-t.re, t.im));
        let a1 = Butterfly4::apply(handle, [input[0], input[2], input[4], input[6]], forward);
        let mut b1 = Butterfly4::apply(handle, [input[1], input[3], input[5], input[7]], forward);
        b1[1] = b1[1] * twiddle;
        b1[2] = if forward {
            b1[2].mul_neg_i()
        } else {
            b1[2].mul_i()
        };
        b1[3] = b1[3] * twiddle_neg;
        let a2 = Butterfly2::apply(handle, [a1[0], b1[0]], forward);
        let b2 = Butterfly2::apply(handle, [a1[1], b1[1]], forward);
        let c2 = Butterfly2::apply(handle, [a1[2], b1[2]], forward);
        let d2 = Butterfly2::apply(handle, [a1[3], b1[3]], forward);
        [a2[0], b2[0], c2[0], d2[0], a2[1], b2[1], c2[1], d2[1]]
    }
}

#[inline(always)]
pub(crate) fn apply_butterfly<T, L, B>(
    _butterfly: B,
    handle: L,
    input: &[num_complex::Complex<T>],
    output: &mut [num_complex::Complex<T>],
    size: usize,
    stride: usize,
    cached_twiddles: &[num_complex::Complex<T>],
    forward: bool,
    wide: bool,
) where
    T: FftFloat,
    L: Loader<num_complex::Complex<T>>,
    B: Butterfly<T, L::Vector>,
    L::Vector: Complex<T>,
{
    let m = size / B::radix();

    // Load twiddle factors
    if wide {
        let full_count = (stride - 1) / L::Vector::width() * L::Vector::width();
        let final_offset = stride - L::Vector::width();
        for i in 0..m {
            let twiddles = {
                let mut twiddles = B::make_buffer(handle);
                for k in 1..B::radix() {
                    twiddles.as_mut()[k] = handle
                        .splat(unsafe { cached_twiddles.as_ptr().add(i * B::radix() + k).read() });
                }
                twiddles
            };

            // Loop over full vectors, with a final overlapping vector
            for j in (0..full_count)
                .step_by(L::Vector::width())
                .chain(core::iter::once(final_offset))
            {
                // Load full vectors
                let mut scratch = B::make_buffer(handle);
                let load = unsafe { input.as_ptr().add(j + stride * i) };
                for k in 0..B::radix() {
                    scratch.as_mut()[k] = unsafe { handle.read_ptr(load.add(stride * k * m)) };
                }

                // Butterfly with optional twiddles
                scratch = B::apply(handle, scratch, forward);
                if size != B::radix() {
                    for k in 1..B::radix() {
                        scratch.as_mut()[k] *= twiddles.as_ref()[k];
                    }
                }

                // Store full vectors
                let store = unsafe { output.as_mut_ptr().add(j + B::radix() * stride * i) };
                for k in 0..B::radix() {
                    unsafe { scratch.as_ref()[k].write_ptr(store.add(stride * k)) };
                }
            }
        }
    } else {
        for i in 0..m {
            let twiddles = {
                let mut twiddles = B::make_buffer(handle);
                for k in 1..B::radix() {
                    twiddles.as_mut()[k] = handle
                        .splat(unsafe { cached_twiddles.as_ptr().add(i * B::radix() + k).read() });
                }
                twiddles
            };

            let load = unsafe { input.as_ptr().add(stride * i) };
            let store = unsafe { output.as_mut_ptr().add(B::radix() * stride * i) };
            for j in 0..stride {
                // Load a single value
                let mut scratch = B::make_buffer(handle);
                for k in 0..B::radix() {
                    scratch.as_mut()[k] =
                        handle.splat(unsafe { load.add(stride * k * m + j).read() });
                }

                // Butterfly with optional twiddles
                scratch = B::apply(handle, scratch, forward);
                if size != B::radix() {
                    for k in 1..B::radix() {
                        scratch.as_mut()[k] *= twiddles.as_ref()[k];
                    }
                }

                // Store a single value
                for k in 0..B::radix() {
                    unsafe {
                        store
                            .add(stride * k + j)
                            .write(scratch.as_ref()[k].as_slice()[0])
                    };
                }
            }
        }
    }
}

macro_rules! implement {
    // the handle must be passed in due to something with macro hygiene
    {
        $handle:ident, $name:ident, $butterfly:ident
    } => {
        paste::item_with_macros! {
            implement! { @impl $handle, [<$name _wide_fwd_f32>], $butterfly, f32, true, true }
            implement! { @impl $handle, [<$name _wide_inv_f32>], $butterfly, f32, true, false }
            implement! { @impl $handle, [<$name _narrow_fwd_f32>], $butterfly, f32, false, true }
            implement! { @impl $handle, [<$name _narrow_inv_f32>], $butterfly, f32, false, false }
            implement! { @impl $handle, [<$name _wide_fwd_f64>], $butterfly, f64, true, true }
            implement! { @impl $handle, [<$name _wide_inv_f64>], $butterfly, f64, true, false }
            implement! { @impl $handle, [<$name _narrow_fwd_f64>], $butterfly, f64, false, true }
            implement! { @impl $handle, [<$name _narrow_inv_f64>], $butterfly, f64, false, false }
        }
    };
    {
        @impl $handle:ident, $name:ident, $butterfly:ident, $type:ty, $wide:expr, $forward:expr
    } => {
        #[safe_simd::dispatch($handle)]
        pub(crate) fn $name(
            input: &[num_complex::Complex<$type>],
            output: &mut [num_complex::Complex<$type>],
            size: usize,
            stride: usize,
            cached_twiddles: &[num_complex::Complex<$type>],
        ) {
            apply_butterfly(
                $butterfly,
                $handle,
                input,
                output,
                size,
                stride,
                cached_twiddles,
                $forward,
                $wide,
            );
        }
    }
}

implement! { handle, radix2, Butterfly2 }
implement! { handle, radix3, Butterfly3 }
implement! { handle, radix4, Butterfly4 }
implement! { handle, radix8, Butterfly8 }
