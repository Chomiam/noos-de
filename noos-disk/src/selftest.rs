//! `noos-disk --selftest /dev/loopN` : enchaîne les opérations destructives sur une image
//! disque de test (jamais sur un vrai disque : refusé si ce n'est pas un périphérique loop).

use std::{cell::RefCell, rc::Rc};

use gtk::glib;
use noos_disk::{ops, udisks};

type Step = Box<dyn FnOnce(Rc<dyn Fn(Result<(), String>)>)>;

pub fn run(dev: &str) -> i32 {
    if !dev.starts_with("/dev/loop") {
        eprintln!("refusé : seulement sur /dev/loopN");
        return 2;
    }
    let conn = udisks::system_bus().unwrap();
    let ml = glib::MainLoop::new(None, false);
    let model = || udisks::load(&udisks::system_bus().unwrap()).unwrap();
    let whole = |m: &udisks::Model| m.blocks.iter().find(|b| b.device == dev_s() && b.partition.is_none()).cloned().unwrap();
    thread_local!(static DEV: RefCell<String> = const { RefCell::new(String::new()) });
    fn dev_s() -> String {
        DEV.with(|d| d.borrow().clone())
    }
    DEV.with(|d| *d.borrow_mut() = dev.to_string());
    let part = move |m: &udisks::Model, n: u32| m.blocks.iter().find(|b| b.partition.as_ref().is_some_and(|p| p.number == n && m.block(&p.table).is_some_and(|t| t.device == dev_s()))).cloned();

    let c = conn.clone();
    let steps: Vec<(&str, Step)> = vec![
        ("table GPT", Box::new({ let c = c.clone(); move |k| { let w = whole(&model()); ops::format(&c, &w.path, "gpt", "", false, move |r| k(r.map(|_| ()))) } })),
        ("partition exFAT 200 Mo", Box::new({ let c = c.clone(); move |k| { let w = whole(&model()); ops::create_partition(&c, &w.path, "gpt", 1 << 20, 200 << 20, "", "exfat", "TESTEXFAT", move |r| k(r.map(|_| ()))) } })),
        ("partition NTFS (reste)", Box::new({ let c = c.clone(); move |k| { let w = whole(&model()); ops::create_partition(&c, &w.path, "gpt", 202 << 20, w.size - (204 << 20), "", "ntfs", "Données NTFS", move |r| k(r.map(|_| ()))) } })),
        ("renommer exFAT", Box::new({ let c = c.clone(); move |k| { let p = part(&model(), 1).unwrap(); ops::set_label(&c, &p.path, "Renommée", move |r| k(r.map(|_| ()))) } })),
        ("monter NTFS", Box::new({ let c = c.clone(); move |k| { let p = part(&model(), 2).unwrap(); ops::mount(&c, &p.path, move |r| { if let Ok(mp) = &r { println!("    monté sur {mp}"); let f = format!("{mp}/essai.txt"); println!("    écriture : {:?}", std::fs::write(&f, "NOOS").map(|_| std::fs::read_to_string(&f).unwrap_or_default())); } k(r.map(|_| ())) }) } })),
        ("démonter NTFS", Box::new({ let c = c.clone(); move |k| { let p = part(&model(), 2).unwrap(); ops::unmount(&c, &p.path, move |r| k(r.map(|_| ()))) } })),
        ("vérifier NTFS", Box::new({ let c = c.clone(); move |k| { let p = part(&model(), 2).unwrap(); ops::check(&c, &p.path, false, move |r| { println!("    cohérent : {r:?}"); k(r.map(|_| ())) }) } })),
        ("supprimer la partition 1", Box::new({ let c = c.clone(); move |k| { let p = part(&model(), 1).unwrap(); ops::delete_partition(&c, &p.path, move |r| k(r.map(|_| ()))) } })),
    ];
    let failed = Rc::new(RefCell::new(0));
    fn next(mut steps: Vec<(&'static str, Step)>, ml: glib::MainLoop, failed: Rc<RefCell<i32>>) {
        if steps.is_empty() {
            ml.quit();
            return;
        }
        let (name, f) = steps.remove(0);
        let ml2 = ml.clone();
        let failed2 = failed.clone();
        let steps = RefCell::new(Some(steps));
        f(Rc::new(move |r| {
            match &r {
                Ok(()) => println!("OK   {name}"),
                Err(e) => {
                    println!("ÉCHEC {name} : {e}");
                    *failed2.borrow_mut() += 1;
                }
            }
            if let Some(s) = steps.borrow_mut().take() {
                let (ml3, f3) = (ml2.clone(), failed2.clone());
                glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || next(s, ml3, f3));
            }
        }));
    }
    next(steps, ml.clone(), failed.clone());
    ml.run();
    let m = model();
    for b in m.blocks.iter().filter(|b| b.partition.as_ref().is_some_and(|p| m.block(&p.table).is_some_and(|t| t.device == dev))) {
        println!("état final : {} « {} » {} {}", b.device, b.id_label, udisks::fs_name(&b.id_type), udisks::human(b.size));
    }
    let n = *failed.borrow();
    n
}
