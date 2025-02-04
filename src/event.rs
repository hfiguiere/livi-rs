use crate::error::EventError;
use lv2_raw::LV2Atom;
use std::convert::TryFrom;
use std::fmt::Debug;
use std::marker::PhantomData;

/// A builder for a single atom event. The max size of the data contained in the
/// event is `MAX_SIZE`.
#[repr(packed)]
pub struct LV2AtomEventBuilder<const MAX_SIZE: usize> {
    /// The atom event.
    event: lv2_raw::LV2AtomEvent,
    /// The data for the atom event.
    _data: [u8; MAX_SIZE],
}

impl<const MAX_SIZE: usize> LV2AtomEventBuilder<MAX_SIZE> {
    /// Create a new atom event with the given time and type.
    ///
    /// # Note
    /// If `data` is of type `[u8; MAX_SIZE]`, then consider using `new_full`.
    ///
    /// # Errors
    /// Returns an error if the size of the buffer is greater than `MAX_SIZE`.
    pub fn new(
        time_in_frames: i64,
        my_type: lv2_raw::LV2Urid,
        data: &[u8],
    ) -> Result<LV2AtomEventBuilder<MAX_SIZE>, EventError> {
        let mut buffer = [0; MAX_SIZE];
        if data.len() > buffer.len() {
            return Err(EventError::DataTooLarge {
                max_supported_size: MAX_SIZE,
                actual_size: data.len(),
            });
        }
        buffer[0..data.len()].copy_from_slice(data);
        Ok(LV2AtomEventBuilder {
            event: lv2_raw::LV2AtomEvent {
                time_in_frames,
                body: LV2Atom {
                    size: u32::try_from(data.len()).expect("Size exceeds u32 capacity."),
                    mytype: my_type,
                },
            },
            _data: buffer,
        })
    }

    /// Create a new atom event with the given data.
    pub fn new_full(
        time_in_frames: i64,
        my_type: lv2_raw::LV2Urid,
        data: [u8; MAX_SIZE],
    ) -> LV2AtomEventBuilder<MAX_SIZE> {
        LV2AtomEventBuilder {
            event: lv2_raw::LV2AtomEvent {
                time_in_frames,
                body: LV2Atom {
                    size: MAX_SIZE as u32,
                    mytype: my_type,
                },
            },
            _data: data,
        }
    }

    /// Create a new midi event.
    ///
    /// This is equivalent to `new` but exists to make it more obvious how to
    /// build midi events.
    ///
    /// # Errors
    /// Returns an error if data cannot fit within `MAX_SIZE`.
    pub fn new_midi(
        time_in_frames: i64,
        midi_uri: lv2_raw::LV2Urid,
        data: &[u8],
    ) -> Result<LV2AtomEventBuilder<MAX_SIZE>, EventError> {
        LV2AtomEventBuilder::<MAX_SIZE>::new(time_in_frames, midi_uri, data)
    }

    /// Return a pointer to the `LV2AtomEvent` that is immediately followed by
    /// its data.
    #[must_use]
    pub fn as_ptr(&self) -> *const lv2_raw::LV2AtomEvent {
        let ptr = self as *const LV2AtomEventBuilder<MAX_SIZE>;
        ptr.cast()
    }
}

impl<const MAX_SIZE: usize> Debug for LV2AtomEventBuilder<MAX_SIZE> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LV2AtomEventBuilder").finish()
    }
}

/// An atom sequence.
pub struct LV2AtomSequence {
    atom_sequence_urid: lv2_raw::LV2Urid,
    atom_chunk_urid: lv2_raw::LV2Urid,
    buffer: Vec<u8>,
}

impl LV2AtomSequence {
    /// Create a new sequence with a capacity to hold `capacity` bytes.
    ///
    /// If `capacity` is too small to hold the header, than it is increased to
    /// the minimum allowable size which is `16` bytes.
    ///
    /// In practice you actually get less usable data than `capacity` because
    /// along with the header taking a couple bytes, all additional events are
    /// aligned to 8 bytes which means the sizes are always rounded up to the
    /// next multiple of 8.
    #[must_use]
    pub fn new(features: &crate::Features, capacity: usize) -> LV2AtomSequence {
        let mut seq = LV2AtomSequence {
            atom_sequence_urid: features.urid(
                std::ffi::CStr::from_bytes_with_nul(b"http://lv2plug.in/ns/ext/atom#Sequence\0")
                    .unwrap(),
            ),
            atom_chunk_urid: features.urid(
                std::ffi::CStr::from_bytes_with_nul(b"http://lv2plug.in/ns/ext/atom#Chunk\0")
                    .unwrap(),
            ),
            buffer: vec![0; capacity + std::mem::size_of::<lv2_raw::LV2AtomSequence>()],
        };
        seq.clear();
        seq
    }

    /// Clear all events in the sequence.
    pub fn clear(&mut self) {
        unsafe {
            let seq = self.as_mut_ptr();
            (*seq).atom.mytype = self.atom_sequence_urid;
            (*seq).atom.size = std::mem::size_of::<lv2_raw::LV2AtomSequenceBody>() as u32;
        }
    }

    /// Clear all events and designate the sequence as a chunk.
    pub fn clear_as_chunk(&mut self) {
        let capacity = self.capacity() as u32;
        unsafe {
            let seq = self.as_mut_ptr();
            (*seq).atom.mytype = self.atom_chunk_urid;
            (*seq).atom.size = capacity;
        }
    }

    /// Append an event to the sequence. If there is no capacity for it, then it
    /// will not be appended.
    ///
    /// # Errors
    /// Returns an error if the event could not be pushed to the sequence.
    pub fn push_event<const MAX_SIZE: usize>(
        &mut self,
        event: &LV2AtomEventBuilder<MAX_SIZE>,
    ) -> Result<(), EventError> {
        let event_size =
            std::mem::size_of::<lv2_raw::LV2AtomEvent>() as u32 + event.event.body.size;
        let sequence = unsafe { &mut *self.as_mut_ptr() };
        // This size includes the atom sequence header.
        let current_sequence_size =
            std::mem::size_of_val(&sequence.atom) as u32 + sequence.atom.size;
        if (self.buffer.len() as u32) < current_sequence_size + event_size {
            return Err(EventError::SequenceFull {
                capacity: self.capacity() as usize,
                requested: (current_sequence_size + event_size) as usize,
            });
        }
        let end = unsafe { lv2_raw::lv2_atom_sequence_end(&sequence.body, sequence.atom.size) }
            as *mut lv2_raw::LV2AtomEvent;
        let src_ptr: *const u8 = event.as_ptr().cast();
        let dst_ptr: *mut u8 = end.cast();
        unsafe { std::ptr::copy_nonoverlapping(src_ptr, dst_ptr, event_size as usize) };
        // This size only includes the sequencey body.
        sequence.atom.size += lv2_raw::lv2_atom_pad_size(event_size);
        Ok(())
    }

    /// Push a new midi event into the sequence. The `midi_data` must be of size
    /// `MAX_SIZE` or smaller. If this is not the case, an error is returned.
    ///
    /// # Errors
    /// Returns an error if the midi data is too large.
    pub fn push_midi_event<const MAX_SIZE: usize>(
        &mut self,
        time_in_frames: i64,
        midi_uri: lv2_raw::LV2Urid,
        data: &[u8],
    ) -> Result<(), EventError> {
        let event = LV2AtomEventBuilder::<MAX_SIZE>::new_midi(time_in_frames, midi_uri, data)?;
        self.push_event(&event)
    }

    /// Return a pointer to the underlying data.
    #[must_use]
    pub fn as_ptr(&self) -> *const lv2_raw::LV2AtomSequence {
        self.buffer.as_ptr().cast()
    }

    /// Return a mutable pointer to the underlying data.
    #[must_use]
    pub fn as_mut_ptr(&mut self) -> *mut lv2_raw::LV2AtomSequence {
        self.buffer.as_mut_ptr().cast()
    }

    /// Get the capacity of the sequence.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.buffer.len() - std::mem::size_of::<lv2_raw::LV2AtomSequence>()
    }

    /// Get the current size of the sequence in bytes.
    #[must_use]
    pub fn size(&self) -> usize {
        let raw = unsafe { self.as_ptr().as_ref().unwrap() };
        let header_size = std::mem::size_of_val(&raw.atom);
        let body_size = raw.atom.size as usize;
        header_size + body_size
    }

    /// Iterate over all events (and event data) in the sequence.
    ///
    /// # Panics
    /// Panics if the underlying sequence is not well formed.
    #[must_use]
    pub fn iter(&self) -> LV2AtomSequenceIter<'_> {
        unsafe {
            let seq = self.as_ptr();
            // Only sequences can be iterated over. Chunks are expected to be
            // passed to plugins and mutated into sequences.
            if (*seq).atom.mytype != self.atom_sequence_urid {
                return LV2AtomSequenceIter {
                    _sequence: PhantomData,
                    body: std::ptr::null(),
                    size: 0,
                    next: std::ptr::null(),
                };
            }
        }
        let body = unsafe { &self.as_ptr().as_ref().unwrap().body };
        let size = unsafe { self.as_ptr().as_ref().unwrap().atom.size };
        let begin = unsafe { lv2_raw::lv2_atom_sequence_begin(body) };
        LV2AtomSequenceIter {
            _sequence: PhantomData,
            body,
            size,
            next: begin,
        }
    }
}

impl Debug for LV2AtomSequence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lv2AtomSequence")
            .field("capacity", &self.capacity())
            .field("size", &self.size())
            .finish()
    }
}

/// An iterator of an `LV2AtomSequence`.
#[derive(Clone)]
pub struct LV2AtomSequenceIter<'a> {
    _sequence: PhantomData<&'a LV2AtomSequence>,
    body: *const lv2_raw::LV2AtomSequenceBody,
    size: u32,
    next: *const lv2_raw::LV2AtomEvent,
}

impl<'a> Iterator for LV2AtomSequenceIter<'a> {
    type Item = LV2AtomEventWithData<'a>;

    fn next(&mut self) -> Option<LV2AtomEventWithData<'a>> {
        if self.size == 0 {
            return None;
        }
        let is_end = unsafe { lv2_raw::lv2_atom_sequence_is_end(self.body, self.size, self.next) };
        if is_end {
            return None;
        }
        let next_ptr = self.next;
        let next = unsafe { next_ptr.as_ref() }?;
        let data_ptr: *const u8 = unsafe { next_ptr.offset(1) }.cast();
        let data_size = next.body.size as usize;
        self.next = unsafe { lv2_raw::lv2_atom_sequence_next(self.next) };
        Some(LV2AtomEventWithData {
            event: next,
            data: unsafe { std::slice::from_raw_parts(data_ptr, data_size) },
        })
    }
}

impl<'a> Debug for LV2AtomSequenceIter<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

/// Contains an `LV2AtomEvent` and its data.
///
/// # Note
/// This type can not usually be used as a direct substitute for `LV2AtomEvent`
/// since it does not guarantee that `event` and `data` are linked together
/// properly in terms of pointers and data layout.
#[derive(Clone)]
pub struct LV2AtomEventWithData<'a> {
    pub event: &'a lv2_raw::LV2AtomEvent,
    pub data: &'a [u8],
}

impl<'a> Debug for LV2AtomEventWithData<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LV2AtomEventWithData")
            .field("time_in_frames", &self.event.time_in_frames)
            .field("my_type", &self.event.body.mytype)
            .field("size", &self.event.body.size)
            .field("data", &self.data)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lazy_static::lazy_static;
    use std::sync::Arc;

    lazy_static! {
        static ref TEST_WORLD: crate::World = crate::World::new();
    }

    fn test_features() -> Arc<crate::Features> {
        TEST_WORLD.build_features(crate::features::FeaturesBuilder {
            min_block_length: 1024,
            max_block_length: 1024,
        })
    }

    #[test]
    fn test_sequence_push_events_and_iter_events() {
        let mut sequence = LV2AtomSequence::new(&test_features(), 4096);
        let event = LV2AtomEventBuilder::<8>::new(0, 0, &[0, 10, 20, 30, 40, 50, 60, 70]).unwrap();
        for _ in 0..10 {
            sequence.push_event(&event).unwrap();
        }
        assert_eq!(10, sequence.iter().count());
        for event in sequence.iter() {
            assert_eq!(event.data, &[0, 10, 20, 30, 40, 50, 60, 70]);
        }
    }

    #[test]
    fn test_sequence_push_events_fails_after_reaching_capacity() {
        // Keep it aligned to 8 bytes to prevent wasting capacity due to
        // padding.
        let event_data = [0; 8];
        let event_size = std::mem::size_of::<lv2_raw::LV2AtomEvent>() + event_data.len();
        assert_eq!(event_size, 24);
        let event = LV2AtomEventBuilder::new_full(0, 0, event_data);

        let events_to_push = 1_000;
        let capacity = events_to_push * event_size;
        let mut sequence = LV2AtomSequence::new(&test_features(), capacity);
        for _ in 0..events_to_push {
            sequence.push_event(&event).unwrap();
        }

        assert_eq!(
            sequence.push_event(&event).err(),
            Some(EventError::SequenceFull {
                capacity,
                requested: 24040,
            })
        );
    }

    #[test]
    fn test_sequence_iter_is_stable() {
        let data = [0; 32];
        for data_size in 0..32 {
            let event = LV2AtomEventBuilder::<32>::new(0, 0, &data[..data_size]).unwrap();
            for capacity in 0..1024 {
                let mut sequence = LV2AtomSequence::new(&test_features(), capacity);
                while sequence.push_event(&event).is_ok() {}
                for event in sequence.iter() {
                    assert_eq!(event.data, &data[..data_size]);
                }
            }
        }
    }

    #[test]
    fn test_clear() {
        let mut sequence = LV2AtomSequence::new(&test_features(), 1024);

        sequence
            .push_event(&LV2AtomEventBuilder::new_full(0, 0, [1, 2, 3]))
            .unwrap();
        assert_eq!(sequence.iter().count(), 1);

        sequence.clear();
        assert_eq!(sequence.iter().count(), 0);
    }

    #[test]
    fn test_clear_as_chunk() {
        let mut sequence = LV2AtomSequence::new(&test_features(), 1024);

        sequence
            .push_event(&LV2AtomEventBuilder::new_full(0, 0, [1, 2, 3]))
            .unwrap();
        assert_eq!(sequence.iter().count(), 1);

        sequence.clear_as_chunk();
        assert_eq!(sequence.iter().count(), 0);
    }
}
