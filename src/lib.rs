use eldenring::{
    cs::{CSEventFlagMan, CSTaskGroupIndex, CSTaskImp, EventFlag, WorldChrMan},
    fd4::FD4TaskData,
    param::SP_EFFECT_PARAM_ST,
    util::system::wait_for_system_init,
};
use fromsoftware_shared::{program::Program, task::*, FromStatic};
use pelite::{
    image::IMAGE_SCN_MEM_EXECUTE,
    pattern,
    pe32::headers::SectionHeader,
    pe64::{Pe, PeObject, PeView},
};
use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::Write,
    ptr::read_unaligned,
    time::Duration,
};

const STARLIGHT_SHARD_SPEFFECT_ID: i32 = 501290;
const STARLIGHT_SHARD_SPEFFECT_EVENT_FLAG: u32 = 366520000;

const SPEFFECTS_INDEX: usize = 15;

const PARAM_BASE_PATTERN_STR: &str = "48 8B 0D ? ? ? ? ? ? ? ? ? ? ? E8 ? ? ? ? 48 85 C0 0F 84 ? ? ? ? 48 8B 80 80 00 00 00 48 8B 90 80 00 00 00";

const OFFSET: usize = 3;
const ADDITIONAL: usize = 7;

fn log(s: &str) {
    let now = chrono::Local::now()
        .format("%y.%m.%d | %H:%M:%S.%3f")
        .to_string();
    let _ = OpenOptions::new()
        .create(true)
        .append(true)
        .open(r"C:\Program Files (x86)\Steam\steamapps\common\ELDEN RING\Game\mod_dll\starlight-shards-as-rune-arcs\logs.log")
        .map(|mut f| writeln!(f, "[{}]: {}", now, s))
        .unwrap();
}

fn get_pe_view() -> PeView<'static> {
    let pe_view = match Program::current() {
        Program::Mapping(mapping) => mapping,
        Program::File(file) => PeView::from_bytes(file.image()).unwrap()
    };

    pe_view
}

fn get_executable_header(pe: PeView<'_>) -> &SectionHeader {
    let executable_header = match pe
        .section_headers()
        .iter()
        .find(|h| &h.Characteristics & IMAGE_SCN_MEM_EXECUTE != 0)
    {
        Some(h) => {
            log("Executable header found");
            h
        }
        None => {
            log("Executable header not found");
            panic!()
        }
    };
    executable_header
}

fn find_param_base_rva(pe_view: PeView) -> usize {
    let pattern = match pattern::parse(PARAM_BASE_PATTERN_STR) {
        Ok(p) => p,
        Err(e) => {
            log(&format!("pattern parse error: {}", e.to_string()));
            panic!()
        }
    };

    let executable_header = get_executable_header(pe_view);
    let scanner = pe_view.scanner();

    let mut matched_rva = [0; 8];
    let mut matches = scanner.matches(&*pattern, executable_header.file_range());

    if !matches.next(&mut matched_rva) {
        log(&format!("No RVA found for pattern {:?}", PARAM_BASE_PATTERN_STR));
        panic!()
    }

    let base_param_ptr_rva = matched_rva[0] as usize;
    log(&format!("Base param RVA {:?}", base_param_ptr_rva));

    base_param_ptr_rva
}

fn get_param_base_ptr(base: *const u8, base_param_rva: usize) -> *const u64 {
    unsafe {
        let base_param_ptr_va: *const u8 = base.add(base_param_rva);
        log(&format!("Base param pointer without offset VA [{:?}]", base_param_ptr_va));

        // readInteger(foundaddr+3,true) where true = signed
        let offset_value: i32 = read_unaligned(base_param_ptr_va.add(OFFSET) as *const i32);
        log(&format!("Offset value for base param pointer [{:?}]", offset_value));

        // foundaddr+7+readInteger(foundaddr+3,true)
        let base_param_ptr_va: *const u64 = base_param_ptr_va.add(ADDITIONAL).offset(offset_value as isize) as *const u64;
        log(&format!("Base param pointer with offset VA [{:?}]", base_param_ptr_va));

        base_param_ptr_va
    }
}

fn get_param_speffect_ptr(param_base_ptr: *const u64) -> *const u64 {
    unsafe {
        // readQword(GetParamBasePtr()) where GetParamBasePtr() is foundaddr+7+readInteger(foundaddr+3,true)
        let base_param_va: *const u64 = read_unaligned(param_base_ptr) as *const u64;
        log(&format!("Base param VA [{:?}]", base_param_va));

        // local hdr=readQword(ParamBase+Index*72+0x88)
        let hdr: *const u64 = read_unaligned((base_param_va as *const u8).add(SPEFFECTS_INDEX * 72 + 0x88) as *const u64) as *const u64;
        log(&format!("Header [{:?}]", hdr));

        // readQword(hdr+0x80)
        let param_goods_ptr: *const u64 = read_unaligned((hdr as *const u8).add(0x80) as *const u64) as *const u64;
        log(&format!("readQword(hdr+0x80) [{:?}]", param_goods_ptr));

        // readQword(readQword(hdr+0x80)+0x80)
        let param_goods_ptr: *const u64 = read_unaligned((param_goods_ptr as *const u8).add(0x80) as *const u64) as *const u64;
        log(&format!("readQword(readQword(hdr+0x80)+0x80) [{:?}]", param_goods_ptr));

        param_goods_ptr
    }
}

fn get_speffect_list_size(param_goods_ptr: *const u64) -> u16 {
    unsafe {
        // local n = readSmallInteger(TableBase + 10)
        let table_size: u16 = read_unaligned((param_goods_ptr as *const u8).add(10) as *const u16);
        log(&format!("Table size: {}", table_size));
        table_size
    }
}

fn form_speffect_map(param_goods_ptr: *const u64, param_goods_list_size: u16) -> HashMap<i32, *const u64> {
    // tbl = {}
    let mut goods_map: HashMap<i32, *const u64> = HashMap::new();

    // for i = 0, n - 1 do
    // tbl[readInteger(TableBase + 64 + 24 * i)] = TableBase + readInteger(TableBase + 64 + 24 * i + 8)
    // end
    for i in 0..param_goods_list_size {
        unsafe {
            let item_id: i32 = read_unaligned((param_goods_ptr as *const u8).add(64 + 24 * i as usize) as *const i32);
            let item_value: i32 = read_unaligned((param_goods_ptr as *const u8).add(64 + 24 * i as usize + 8) as *const i32);
            let item_value = (param_goods_ptr as *const u8).add(item_value as usize) as *const u64;

            goods_map.insert(item_id, item_value);
        }
    }

    log("Goods table formed");

    goods_map
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DllMain(_hmodule: u64, reason: u32) -> bool {
    if reason != 1 {
        return true;
    }

    std::thread::spawn(|| {
        wait_for_system_init(&Program::current(), Duration::MAX)
            .expect("Timeout waiting for system init");

        let mut is_starlight_shard_modded = false;

        let cs_task = unsafe { CSTaskImp::instance().unwrap() };

        cs_task.run_recurring(
            move |_: &FD4TaskData| {
                let Some(main_player) = unsafe { WorldChrMan::instance() }
                    .ok()
                    .and_then(|world_chr_man| world_chr_man.main_player.as_mut())
                else {
                    return
                };
                let Ok(event_flags_manager) = (unsafe { CSEventFlagMan::instance() })
                else {
                    return
                };

                let is_player_have_starlight_shard_speffect = main_player.chr_ins.special_effect.entries()
                    .find(|e| e.param_id == STARLIGHT_SHARD_SPEFFECT_ID)
                    .is_some();
                let is_player_alive = main_player.chr_ins.module_container.data.hp > 0;
                let is_starlight_shards_event_flag_active = unsafe { event_flags_manager.virtual_memory_flag.flag_blocks.as_mut().unwrap().get(EventFlag::from(STARLIGHT_SHARD_SPEFFECT_EVENT_FLAG)) };

                // Run once on game load
                if !is_starlight_shard_modded && is_player_alive {
                    let pe_view: PeView = get_pe_view();
                    let base_param_rva: usize = find_param_base_rva(pe_view);
                    let param_base_ptr: *const u64 = get_param_base_ptr(pe_view.image().as_ptr(), base_param_rva);
                    let param_speffect_ptr: *const u64 = get_param_speffect_ptr(param_base_ptr);
                    let speffect_map: HashMap<i32, *const u64> = form_speffect_map(param_speffect_ptr, get_speffect_list_size(param_speffect_ptr));

                    unsafe {
                        let starlight_shard_ptr: *const u64 = *speffect_map.get(&STARLIGHT_SHARD_SPEFFECT_ID).unwrap();
                        let starlight_shard_speffect_param: &mut SP_EFFECT_PARAM_ST = &mut *(starlight_shard_ptr as *mut SP_EFFECT_PARAM_ST);

                        starlight_shard_speffect_param.set_effect_endurance(-1.0);
                        log(&format!("Starlight Shard effect endurance changed to {}", starlight_shard_speffect_param.effect_endurance()));

                        is_starlight_shard_modded = true;
                    }
                }

                // Set event flag for persistent state save
                if !is_starlight_shards_event_flag_active && is_player_have_starlight_shard_speffect {
                    unsafe {
                        event_flags_manager.virtual_memory_flag.flag_blocks.as_mut().unwrap().set(EventFlag::from(STARLIGHT_SHARD_SPEFFECT_EVENT_FLAG), true);
                        let x = event_flags_manager.virtual_memory_flag.flag_blocks.as_mut().unwrap().get(EventFlag::from(STARLIGHT_SHARD_SPEFFECT_EVENT_FLAG));
                        log(&format!("Starlight shards event flag set to {}", x));
                    }
                }

                // Apply Starlight Shards effect on game load if it was applied in previous session
                if is_starlight_shards_event_flag_active && is_player_alive && !is_player_have_starlight_shard_speffect {
                    main_player.chr_ins.apply_speffect(STARLIGHT_SHARD_SPEFFECT_ID, true);
                    log("Starlight Shard speffect applied based on the event flag");
                }

                // Remove on death
                if !is_player_alive && is_player_have_starlight_shard_speffect {
                    main_player.chr_ins.remove_speffect(STARLIGHT_SHARD_SPEFFECT_ID);
                    unsafe { event_flags_manager.virtual_memory_flag.flag_blocks.as_mut().unwrap().set(EventFlag::from(STARLIGHT_SHARD_SPEFFECT_EVENT_FLAG), false) };
                    log("Player died, removing Starlight Shard effect and setting event flag to false");
                    return;
                }
            },
            CSTaskGroupIndex::FrameBegin,
        );
    });
    true
}
