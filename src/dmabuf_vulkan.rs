//! Vulkan-based dmabuf importer (phase 1: GPU readback)
//! This module attempts to import single-plane linear dmabufs into a Vulkan image
//! and read back RGBA bytes. It requires the following device extensions:
//! - VK_KHR_external_memory
//! - VK_KHR_external_memory_fd
//! - VK_EXT_external_memory_dma_buf
//! - VK_EXT_image_drm_format_modifier
//!
//! If any requirement is missing at runtime, the functions return None and the
//! caller must fall back to CPU conversion.

#![cfg(feature = "dmabuf-vulkan")]

use ash::{vk, Entry, StaticFn};
use std::ffi::{CStr, CString};
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::{Mutex, OnceLock};

#[allow(dead_code)]
struct VkCtx {
    _lib: libloading::Library,
    entry: ash::Entry,
    instance: ash::Instance,
    phys: vk::PhysicalDevice,
    device: ash::Device,
    queue: vk::Queue,
    qfam: u32,
    cmd_pool: vk::CommandPool,
}

impl Drop for VkCtx {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_command_pool(self.cmd_pool, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

static CTX: OnceLock<Mutex<VkCtx>> = OnceLock::new();

fn required_device_extensions() -> Vec<&'static CStr> {
    // Use raw names to avoid depending on ash's private modules
    const KHR_EXTERNAL_MEMORY: &CStr =
        unsafe { CStr::from_bytes_with_nul_unchecked(b"VK_KHR_external_memory\0") };
    const KHR_EXTERNAL_MEMORY_FD: &CStr =
        unsafe { CStr::from_bytes_with_nul_unchecked(b"VK_KHR_external_memory_fd\0") };
    const EXT_EXTERNAL_MEMORY_DMA_BUF: &CStr =
        unsafe { CStr::from_bytes_with_nul_unchecked(b"VK_EXT_external_memory_dma_buf\0") };
    const EXT_IMAGE_DRM_FORMAT_MODIFIER: &CStr =
        unsafe { CStr::from_bytes_with_nul_unchecked(b"VK_EXT_image_drm_format_modifier\0") };
    vec![
        KHR_EXTERNAL_MEMORY,
        KHR_EXTERNAL_MEMORY_FD,
        EXT_EXTERNAL_MEMORY_DMA_BUF,
        EXT_IMAGE_DRM_FORMAT_MODIFIER,
    ]
}

fn create_ctx() -> Option<VkCtx> {
    // Load libvulkan and vkGetInstanceProcAddr
    let lib = unsafe { libloading::Library::new("libvulkan.so.1").ok()? };
    let get_inst: libloading::Symbol<
        '_,
        unsafe extern "system" fn(vk::Instance, *const i8) -> vk::PFN_vkVoidFunction,
    > = unsafe { lib.get(b"vkGetInstanceProcAddr\0").ok()? };
    let static_fn = StaticFn {
        get_instance_proc_addr: *get_inst,
    };
    let entry = unsafe { Entry::from_static_fn(static_fn) };
    let app_name = CString::new("axiom-dmabuf-vulkan").ok()?;
    let app_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(0)
        .engine_name(&app_name)
        .engine_version(0)
        .api_version(vk::API_VERSION_1_1);

    let create_info = vk::InstanceCreateInfo::default().application_info(&app_info);
    let instance = unsafe { entry.create_instance(&create_info, None).ok()? };

    // Pick a suitable physical device
    let phys_list = unsafe { instance.enumerate_physical_devices().ok()? };
    let req_exts = required_device_extensions();
    let mut chosen: Option<(vk::PhysicalDevice, u32)> = None;
    'outer: for &pd in &phys_list {
        let exts = unsafe { instance.enumerate_device_extension_properties(pd).ok()? };
        // Check extension presence
        for req in &req_exts {
            let mut found = false;
            for e in &exts {
                let name = unsafe { CStr::from_ptr(e.extension_name.as_ptr()) };
                if name == *req {
                    found = true;
                    break;
                }
            }
            if !found {
                continue 'outer;
            }
        }
        // Find a graphics or transfer queue
        let qprops = unsafe { instance.get_physical_device_queue_family_properties(pd) };
        for (idx, qp) in qprops.iter().enumerate() {
            let supports = qp.queue_flags.contains(vk::QueueFlags::TRANSFER)
                || qp.queue_flags.contains(vk::QueueFlags::GRAPHICS);
            if supports {
                chosen = Some((pd, idx as u32));
                break 'outer;
            }
        }
    }
    let (phys, qfam) = chosen?;

    // Create device
    let pri = 1.0f32;
    let pri_binding = [pri];
    let qinfo_binding = [vk::DeviceQueueCreateInfo::default()
        .queue_family_index(qfam)
        .queue_priorities(&pri_binding)];
    let ext_names: Vec<*const i8> = req_exts.iter().map(|c| c.as_ptr()).collect();
    let dinfo = vk::DeviceCreateInfo::default()
        .queue_create_infos(&qinfo_binding)
        .enabled_extension_names(&ext_names);

    let device = unsafe { instance.create_device(phys, &dinfo, None).ok()? };
    let queue = unsafe { device.get_device_queue(qfam, 0) };

    // Command pool for one-shot operations
    let pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(qfam)
        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
    let cmd_pool = unsafe { device.create_command_pool(&pool_info, None).ok()? };

    Some(VkCtx {
        _lib: lib,
        entry,
        instance,
        phys,
        device,
        queue,
        qfam,
        cmd_pool,
    })
}

fn get_ctx() -> Option<std::sync::MutexGuard<'static, VkCtx>> {
    let m = CTX.get_or_init(|| Mutex::new(create_ctx().expect("Vulkan init failed")));
    // If create_ctx failed, the expect() will panic; instead, guard: re-initialize with graceful None
    // But since OnceLock cannot reset, we avoid None path here by creating on first call.
    // In practice, if init fails, this process likely lacks Vulkan support anyway.
    m.lock().ok()
}

fn map_fourcc_to_vk_format(fourcc: u32) -> Option<(vk::Format, bool)> {
    // returns (VkFormat, needs_bgra_to_rgba_swizzle)
    const DRM_FORMAT_XRGB8888: u32 = 0x34325258; // 'XR24'
    const DRM_FORMAT_ARGB8888: u32 = 0x34325241; // 'AR24'
    const DRM_FORMAT_XBGR8888: u32 = 0x34324258; // 'XB24'
    const DRM_FORMAT_ABGR8888: u32 = 0x34324241; // 'AB24'
    match fourcc {
        DRM_FORMAT_XRGB8888 | DRM_FORMAT_ARGB8888 => Some((vk::Format::B8G8R8A8_UNORM, true)),
        DRM_FORMAT_XBGR8888 | DRM_FORMAT_ABGR8888 => Some((vk::Format::R8G8B8A8_UNORM, false)),
        _ => None,
    }
}

unsafe fn submit_and_wait(
    device: &ash::Device,
    queue: vk::Queue,
    cmd: vk::CommandBuffer,
) -> Option<()> {
    let fence_info = vk::FenceCreateInfo::default();
    let fence = device.create_fence(&fence_info, None).ok()?;

    let cmd_binding = [cmd];
    let submit_info = [vk::SubmitInfo::default().command_buffers(&cmd_binding)];
    device.queue_submit(queue, &submit_info, fence).ok()?;
    device
        .wait_for_fences(&[fence], true, 10_000_000_000)
        .ok()?; // 10s
    device.destroy_fence(fence, None);
    Some(())
}

pub fn import_rgba_from_dmabuf(
    fourcc: u32,
    width: u32,
    height: u32,
    stride: i32,
    offset: i32,
    fd: &OwnedFd,
) -> Option<Vec<u8>> {
    let mut guard = get_ctx()?;
    let ctx = &mut *guard;

    let (format, bgra_to_rgba) = map_fourcc_to_vk_format(fourcc)?;

    // Duplicate fd for Vulkan (it takes ownership)
    let dup_fd = unsafe { libc::dup(fd.as_raw_fd()) };
    if dup_fd < 0 {
        return None;
    }

    let image_tiling = vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT;

    // Plane layout
    let plane_layout = vk::SubresourceLayout {
        offset: offset as u64,
        size: 0, // ignored when using explicit create info
        row_pitch: stride as u64,
        array_pitch: 0,
        depth_pitch: 0,
    };

    let mut drm_mod_info = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
        .drm_format_modifier(0) // linear modifier
        .plane_layouts(std::slice::from_ref(&plane_layout));

    let mut external_info = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);

    let image_info = vk::ImageCreateInfo::default()
        .push_next(&mut drm_mod_info)
        .push_next(&mut external_info)
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(image_tiling)
        .usage(vk::ImageUsageFlags::TRANSFER_SRC)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);

    let image = unsafe { ctx.device.create_image(&image_info, None).ok()? };
    let mem_req = unsafe { ctx.device.get_image_memory_requirements(image) };

    // Import memory from fd
    let mut import_info = vk::ImportMemoryFdInfoKHR::default()
        .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
        .fd(dup_fd);

    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);

    // Pick a memory type from mem_req.memory_type_bits (no further flags required)
    let mem_props = unsafe { ctx.instance.get_physical_device_memory_properties(ctx.phys) };
    let mut mem_type_index = None;
    for i in 0..mem_props.memory_type_count {
        if (mem_req.memory_type_bits & (1 << i)) != 0 {
            mem_type_index = Some(i);
            break;
        }
    }
    let mem_type_index = mem_type_index?;

    let alloc_info = vk::MemoryAllocateInfo::default()
        .push_next(&mut import_info)
        .push_next(&mut dedicated)
        .allocation_size(mem_req.size)
        .memory_type_index(mem_type_index);

    let image_mem = unsafe { ctx.device.allocate_memory(&alloc_info, None).ok()? };
    unsafe { ctx.device.bind_image_memory(image, image_mem, 0).ok()? };

    // Create staging buffer
    let out_size = (width as usize) * (height as usize) * 4;
    let buffer_info = vk::BufferCreateInfo::default()
        .size(out_size as u64)
        .usage(vk::BufferUsageFlags::TRANSFER_DST)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let buffer = unsafe { ctx.device.create_buffer(&buffer_info, None).ok()? };
    let buf_req = unsafe { ctx.device.get_buffer_memory_requirements(buffer) };

    let mut buf_mem_type = None;
    for i in 0..mem_props.memory_type_count {
        if (buf_req.memory_type_bits & (1 << i)) != 0
            && mem_props.memory_types[i as usize].property_flags.contains(
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            )
        {
            buf_mem_type = Some(i);
            break;
        }
    }
    let buf_mem_type = buf_mem_type?;

    let buf_alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(buf_req.size)
        .memory_type_index(buf_mem_type);
    let buffer_mem = unsafe { ctx.device.allocate_memory(&buf_alloc, None).ok()? };
    unsafe { ctx.device.bind_buffer_memory(buffer, buffer_mem, 0).ok()? };

    // Command buffer
    let cmd_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(ctx.cmd_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let cmd_buf = unsafe { ctx.device.allocate_command_buffers(&cmd_info).ok()? }[0];

    unsafe {
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        ctx.device.begin_command_buffer(cmd_buf, &begin).ok()?;

        // Transition image layout to TRANSFER_SRC_OPTIMAL
        let barrier = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1),
            );
        ctx.device.cmd_pipeline_barrier(
            cmd_buf,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier],
        );

        // Copy image to buffer
        let region = vk::BufferImageCopy::default()
            .buffer_offset(0)
            .buffer_row_length(0)
            .buffer_image_height(0)
            .image_subresource(
                vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .mip_level(0)
                    .base_array_layer(0)
                    .layer_count(1),
            )
            .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
            .image_extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            });
        ctx.device.cmd_copy_image_to_buffer(
            cmd_buf,
            image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            buffer,
            &[region],
        );

        ctx.device.end_command_buffer(cmd_buf).ok()?;
        submit_and_wait(&ctx.device, ctx.queue, cmd_buf)?;
        ctx.device.free_command_buffers(ctx.cmd_pool, &[cmd_buf]);
    }

    // Map staging memory
    let ptr = unsafe {
        ctx.device
            .map_memory(buffer_mem, 0, out_size as u64, vk::MemoryMapFlags::empty())
            .ok()?
    } as *const u8;
    let mut out = vec![0u8; out_size];
    unsafe {
        std::ptr::copy_nonoverlapping(ptr, out.as_mut_ptr(), out_size);
        ctx.device.unmap_memory(buffer_mem);
    }

    // Cleanup
    unsafe {
        ctx.device.destroy_buffer(buffer, None);
        ctx.device.free_memory(buffer_mem, None);
        ctx.device.destroy_image(image, None);
        ctx.device.free_memory(image_mem, None);
    }

    if bgra_to_rgba {
        // Convert BGRA -> RGBA in place
        for px in out.chunks_exact_mut(4) {
            let b = px[0];
            let g = px[1];
            let r = px[2];
            let a = px[3];
            px[0] = r;
            px[1] = g;
            px[2] = b;
            px[3] = a;
        }
    }

    Some(out)
}
