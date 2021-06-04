/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use super::DeviceError;
use arrayvec::ArrayVec;

pub use gpu_descriptor::DescriptorTotalCount;

pub type DescriptorSet<A> = gpu_descriptor::DescriptorSet<<B as hal::Backend>::DescriptorSet>;

#[derive(Debug)]
pub struct DescriptorAllocator<A: hal::Api>(
    gpu_descriptor::DescriptorAllocator<B::DescriptorPool, B::DescriptorSet>,
);
struct DescriptorDevice<'a, A: hal::Api>(&'a B::Device);

impl<A: hal::Api> DescriptorAllocator<A> {
    pub fn new() -> Self {
        DescriptorAllocator(gpu_descriptor::DescriptorAllocator::new(0))
    }

    pub fn allocate(
        &mut self,
        device: &B::Device,
        layout: &B::DescriptorSetLayout,
        layout_descriptor_count: &DescriptorTotalCount,
        count: u32,
    ) -> Result<Vec<DescriptorSet<A>>, DeviceError> {
        unsafe {
            self.0.allocate(
                &DescriptorDevice::<A>(device),
                layout,
                gpu_descriptor::DescriptorSetLayoutCreateFlags::empty(),
                layout_descriptor_count,
                count,
            )
        }
        .map_err(|err| {
            log::warn!("Descriptor set allocation failed: {}", err);
            DeviceError::OutOfMemory
        })
    }

    pub fn free(&mut self, device: &B::Device, sets: impl IntoIterator<Item = DescriptorSet<A>>) {
        unsafe { self.0.free(&DescriptorDevice::<A>(device), sets) }
    }

    pub fn cleanup(&mut self, device: &B::Device) {
        unsafe { self.0.cleanup(&DescriptorDevice::<A>(device)) }
    }
}

impl<A: hal::Api>
    gpu_descriptor::DescriptorDevice<B::DescriptorSetLayout, B::DescriptorPool, B::DescriptorSet>
    for DescriptorDevice<'_, B>
{
    unsafe fn create_descriptor_pool(
        &self,
        descriptor_count: &DescriptorTotalCount,
        max_sets: u32,
        flags: gpu_descriptor::DescriptorPoolCreateFlags,
    ) -> Result<B::DescriptorPool, gpu_descriptor::CreatePoolError> {
        profiling::scope!("create_descriptor_pool");
        let mut ranges = ArrayVec::<[_; 7]>::new();

        ranges.push(hal::pso::DescriptorRangeDesc {
            ty: hal::pso::DescriptorType::Sampler,
            count: descriptor_count.sampler as _,
        });
        ranges.push(hal::pso::DescriptorRangeDesc {
            ty: hal::pso::DescriptorType::Image {
                ty: hal::pso::ImageDescriptorType::Sampled {
                    with_sampler: false,
                },
            },
            count: descriptor_count.sampled_image as _,
        });
        ranges.push(hal::pso::DescriptorRangeDesc {
            ty: hal::pso::DescriptorType::Image {
                ty: hal::pso::ImageDescriptorType::Storage { read_only: false },
            },
            count: descriptor_count.storage_image as _,
        });
        ranges.push(hal::pso::DescriptorRangeDesc {
            ty: hal::pso::DescriptorType::Buffer {
                ty: hal::pso::BufferDescriptorType::Uniform,
                format: hal::pso::BufferDescriptorFormat::Structured {
                    dynamic_offset: false,
                },
            },
            count: descriptor_count.uniform_buffer as _,
        });
        ranges.push(hal::pso::DescriptorRangeDesc {
            ty: hal::pso::DescriptorType::Buffer {
                ty: hal::pso::BufferDescriptorType::Storage { read_only: false },
                format: hal::pso::BufferDescriptorFormat::Structured {
                    dynamic_offset: false,
                },
            },
            count: descriptor_count.storage_buffer as _,
        });
        ranges.push(hal::pso::DescriptorRangeDesc {
            ty: hal::pso::DescriptorType::Buffer {
                ty: hal::pso::BufferDescriptorType::Uniform,
                format: hal::pso::BufferDescriptorFormat::Structured {
                    dynamic_offset: true,
                },
            },
            count: descriptor_count.uniform_buffer_dynamic as _,
        });
        ranges.push(hal::pso::DescriptorRangeDesc {
            ty: hal::pso::DescriptorType::Buffer {
                ty: hal::pso::BufferDescriptorType::Storage { read_only: false },
                format: hal::pso::BufferDescriptorFormat::Structured {
                    dynamic_offset: true,
                },
            },
            count: descriptor_count.storage_buffer_dynamic as _,
        });
        ranges.retain(|rd| rd.count != 0);

        match hal::device::Device::create_descriptor_pool(
            self.0,
            max_sets as usize,
            ranges.into_iter(),
            hal::pso::DescriptorPoolCreateFlags::from_bits_truncate(flags.bits()),
        ) {
            Ok(pool) => Ok(pool),
            Err(hal::device::OutOfMemory::Host) => {
                Err(gpu_descriptor::CreatePoolError::OutOfHostMemory)
            }
            Err(hal::device::OutOfMemory::Device) => {
                Err(gpu_descriptor::CreatePoolError::OutOfDeviceMemory)
            }
        }
    }

    unsafe fn destroy_descriptor_pool(&self, pool: B::DescriptorPool) {
        profiling::scope!("destroy_descriptor_pool");
        hal::device::Device::destroy_descriptor_pool(self.0, pool);
    }

    unsafe fn alloc_descriptor_sets<'a>(
        &self,
        pool: &mut B::DescriptorPool,
        layouts: impl ExactSizeIterator<Item = &'a B::DescriptorSetLayout>,
        sets: &mut impl Extend<B::DescriptorSet>,
    ) -> Result<(), gpu_descriptor::DeviceAllocationError> {
        use gpu_descriptor::DeviceAllocationError as Dae;
        profiling::scope!("alloc_descriptor_sets");

        match hal::pso::DescriptorPool::allocate(pool, layouts, sets) {
            Ok(()) => Ok(()),
            Err(hal::pso::AllocationError::OutOfMemory(oom)) => Err(match oom {
                hal::device::OutOfMemory::Host => Dae::OutOfHostMemory,
                hal::device::OutOfMemory::Device => Dae::OutOfDeviceMemory,
            }),
            Err(hal::pso::AllocationError::OutOfPoolMemory) => Err(Dae::OutOfPoolMemory),
            Err(hal::pso::AllocationError::FragmentedPool) => Err(Dae::FragmentedPool),
            Err(hal::pso::AllocationError::IncompatibleLayout) => {
                panic!("Incompatible descriptor set layout")
            }
        }
    }

    unsafe fn dealloc_descriptor_sets<'a>(
        &self,
        pool: &mut B::DescriptorPool,
        sets: impl Iterator<Item = B::DescriptorSet>,
    ) {
        profiling::scope!("dealloc_descriptor_sets");
        hal::pso::DescriptorPool::free(pool, sets)
    }
}
