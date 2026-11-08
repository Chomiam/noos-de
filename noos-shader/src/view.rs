//! Rendu d'une animation GLSL (format Shadertoy) dans un `GtkGLArea`.
//!
//! Le code de l'utilisateur (`mainImage`) est enveloppé : en-tête GLSL (ES 3.0 ou 3.3 core
//! selon le contexte), uniformes Shadertoy (`iResolution`, `iTime`, `iTimeDelta`, `iFrame`,
//! `iMouse`, `iDate`, `iChannel0..3` noirs…), onglet « Common », puis `main()`. Un triangle
//! couvre l'écran. Rendu éventuellement à résolution réduite (FBO puis agrandissement
//! linéaire) et images par seconde plafonnées.

use std::{
    cell::{Cell, RefCell},
    ffi::CString,
    rc::Rc,
    sync::Once,
    time::Instant,
};

use gtk::{gdk, glib, prelude::*};

use crate::Source;

static LOAD: Once = Once::new();

fn load_gl() {
    LOAD.call_once(|| {
        // libepoxy (déjà chargée par GTK) expose chaque fonction OpenGL `glXyz` sous la forme
        // d'une variable `epoxy_glXyz` contenant un pointeur de fonction (résolu au premier appel).
        let lib = unsafe { libloading::Library::new("libepoxy.so.0") }.expect("libepoxy.so.0");
        gl::load_with(|name| unsafe {
            let sym = format!("epoxy_{name}\0");
            lib.get::<*const *const std::ffi::c_void>(sym.as_bytes()).map(|s| **s).unwrap_or(std::ptr::null())
        });
        std::mem::forget(lib);
    });
}

const VERTEX: &str = "const vec2 noos_pos[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));\n\
void main() { gl_Position = vec4(noos_pos[gl_VertexID], 0.0, 1.0); }\n";

const UNIFORMS: &str = "uniform vec3 iResolution;\nuniform float iTime;\nuniform float iTimeDelta;\nuniform float iFrameRate;\n\
uniform int iFrame;\nuniform vec4 iMouse;\nuniform vec4 iDate;\nuniform float iSampleRate;\nuniform float iChannelTime[4];\n\
uniform vec3 iChannelResolution[4];\nuniform sampler2D iChannel0;\nuniform sampler2D iChannel1;\nuniform sampler2D iChannel2;\n\
uniform sampler2D iChannel3;\n";

const MAIN: &str = "\nout vec4 noos_FragColor;\nvoid main() {\n    vec4 c = vec4(0.0, 0.0, 0.0, 1.0);\n\
    mainImage(c, gl_FragCoord.xy);\n    noos_FragColor = vec4(c.rgb, 1.0);\n}\n";

#[derive(Default)]
struct Gl {
    program: u32,
    vao: u32,
    black: u32,
    fbo: u32,
    tex: u32,
    fbo_size: (i32, i32),
    loc: [i32; 9],
}

struct Inner {
    gl: RefCell<Gl>,
    source: RefCell<Option<Source>>,
    dirty: Cell<bool>,
    start: Cell<Instant>,
    last: Cell<Instant>,
    frame: Cell<i32>,
    fps: Cell<u32>,
    scale: Cell<f32>,
    error: RefCell<Option<Box<dyn Fn(&str)>>>,
}

/// Zone animée (se comporte comme un widget : `widget()`).
#[derive(Clone)]
pub struct ShaderView {
    area: gtk::GLArea,
    inner: Rc<Inner>,
}

impl ShaderView {
    pub fn new() -> Self {
        let area = gtk::GLArea::new();
        area.set_auto_render(false);
        area.set_has_depth_buffer(false);
        area.set_hexpand(true);
        area.set_vexpand(true);
        let inner = Rc::new(Inner {
            gl: RefCell::new(Gl::default()),
            source: RefCell::new(None),
            dirty: Cell::new(false),
            start: Cell::new(Instant::now()),
            last: Cell::new(Instant::now()),
            frame: Cell::new(0),
            fps: Cell::new(30),
            scale: Cell::new(1.0),
            error: RefCell::new(None),
        });
        {
            let inner = inner.clone();
            area.connect_realize(move |a| {
                a.make_current();
                if a.error().is_some() {
                    return;
                }
                load_gl();
                let mut g = inner.gl.borrow_mut();
                unsafe {
                    gl::GenVertexArrays(1, &mut g.vao);
                    gl::GenTextures(1, &mut g.black);
                    gl::BindTexture(gl::TEXTURE_2D, g.black);
                    let px: [u8; 4] = [0, 0, 0, 255];
                    gl::TexImage2D(gl::TEXTURE_2D, 0, gl::RGBA as i32, 1, 1, 0, gl::RGBA, gl::UNSIGNED_BYTE, px.as_ptr().cast());
                    gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
                }
                inner.dirty.set(true);
            });
        }
        {
            let inner = inner.clone();
            area.connect_unrealize(move |a| {
                a.make_current();
                if a.error().is_some() {
                    return;
                }
                let mut g = inner.gl.borrow_mut();
                unsafe {
                    if g.program != 0 {
                        gl::DeleteProgram(g.program);
                    }
                    gl::DeleteVertexArrays(1, &g.vao);
                    gl::DeleteTextures(1, &g.black);
                    if g.fbo != 0 {
                        gl::DeleteFramebuffers(1, &g.fbo);
                        gl::DeleteTextures(1, &g.tex);
                    }
                }
                *g = Gl::default();
            });
        }
        {
            let inner = inner.clone();
            area.connect_render(move |a, _ctx| {
                render(a, &inner);
                glib::Propagation::Stop
            });
        }
        // Cadence : une image dès que l'intervalle 1/fps est écoulé.
        {
            let inner = inner.clone();
            area.add_tick_callback(move |a, _| {
                let fps = inner.fps.get().clamp(1, 240);
                if inner.last.get().elapsed().as_secs_f64() >= 1.0 / f64::from(fps) - 0.002 {
                    a.queue_render();
                }
                glib::ControlFlow::Continue
            });
        }
        Self { area, inner }
    }

    pub fn widget(&self) -> &gtk::GLArea {
        &self.area
    }

    pub fn set_source(&self, source: Option<Source>) {
        *self.inner.source.borrow_mut() = source;
        self.inner.dirty.set(true);
        self.inner.start.set(Instant::now());
        self.inner.frame.set(0);
        self.area.queue_render();
    }

    pub fn set_fps(&self, fps: u32) {
        self.inner.fps.set(fps);
    }

    /// Échelle de rendu (0,25 à 1).
    pub fn set_render_scale(&self, s: f32) {
        self.inner.scale.set(s.clamp(0.25, 1.0));
    }

    /// Appelé avec le message du compilateur si l'animation ne compile pas.
    pub fn connect_error(&self, f: impl Fn(&str) + 'static) {
        *self.inner.error.borrow_mut() = Some(Box::new(f));
    }
}

impl Default for ShaderView {
    fn default() -> Self {
        Self::new()
    }
}

fn compile(kind: u32, src: &str) -> Result<u32, String> {
    unsafe {
        let sh = gl::CreateShader(kind);
        let c = CString::new(src.replace('\0', "")).unwrap();
        gl::ShaderSource(sh, 1, &c.as_ptr(), std::ptr::null());
        gl::CompileShader(sh);
        let mut ok = 0;
        gl::GetShaderiv(sh, gl::COMPILE_STATUS, &mut ok);
        if ok == 0 {
            let mut len = 0;
            gl::GetShaderiv(sh, gl::INFO_LOG_LENGTH, &mut len);
            let mut buf = vec![0u8; len.max(1) as usize];
            gl::GetShaderInfoLog(sh, len, std::ptr::null_mut(), buf.as_mut_ptr().cast());
            gl::DeleteShader(sh);
            return Err(String::from_utf8_lossy(&buf).trim_end_matches('\0').trim().to_string());
        }
        Ok(sh)
    }
}

fn build(es: bool, src: &Source) -> Result<(u32, [i32; 9]), String> {
    let header = if es { "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision mediump sampler2D;\n" } else { "#version 330 core\n" };
    let vs = compile(gl::VERTEX_SHADER, &format!("{header}{VERTEX}"))?;
    let fs_src = format!("{header}{UNIFORMS}\n{}\n{}\n{MAIN}", src.common, src.image);
    let fs = match compile(gl::FRAGMENT_SHADER, &fs_src) {
        Ok(f) => f,
        Err(e) => {
            unsafe { gl::DeleteShader(vs) };
            return Err(e);
        }
    };
    unsafe {
        let p = gl::CreateProgram();
        gl::AttachShader(p, vs);
        gl::AttachShader(p, fs);
        gl::LinkProgram(p);
        gl::DeleteShader(vs);
        gl::DeleteShader(fs);
        let mut ok = 0;
        gl::GetProgramiv(p, gl::LINK_STATUS, &mut ok);
        if ok == 0 {
            gl::DeleteProgram(p);
            return Err("édition des liens impossible".into());
        }
        let loc = |n: &str| {
            let c = CString::new(n).unwrap();
            gl::GetUniformLocation(p, c.as_ptr())
        };
        Ok((
            p,
            [
                loc("iResolution"),
                loc("iTime"),
                loc("iTimeDelta"),
                loc("iFrame"),
                loc("iMouse"),
                loc("iDate"),
                loc("iSampleRate"),
                loc("iFrameRate"),
                loc("iChannel0"),
            ],
        ))
    }
}

fn render(area: &gtk::GLArea, inner: &Inner) {
    if area.error().is_some() {
        return;
    }
    let mut g = inner.gl.borrow_mut();
    if inner.dirty.replace(false) {
        unsafe {
            if g.program != 0 {
                gl::DeleteProgram(g.program);
                g.program = 0;
            }
        }
        if let Some(src) = inner.source.borrow().as_ref() {
            let es = area.context().is_some_and(|c| c.api() == gdk::GLAPI::GLES);
            match build(es, src) {
                Ok((p, loc)) => {
                    g.program = p;
                    g.loc = loc;
                }
                Err(e) => {
                    eprintln!("noos-shader : « {} » ne compile pas : {e}", src.name);
                    if let Some(f) = inner.error.borrow().as_ref() {
                        f(&e);
                    }
                }
            }
        }
    }
    let scale_factor = area.scale_factor();
    let (w, h) = (area.width() * scale_factor, area.height() * scale_factor);
    unsafe {
        if g.program == 0 || w <= 0 || h <= 0 {
            gl::ClearColor(0.0, 0.0, 0.0, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);
            return;
        }
        let s = inner.scale.get();
        let (rw, rh) = if s < 0.99 { (((w as f32) * s).max(1.0) as i32, ((h as f32) * s).max(1.0) as i32) } else { (w, h) };
        let mut target = 0;
        gl::GetIntegerv(gl::DRAW_FRAMEBUFFER_BINDING, &mut target);
        let offscreen = (rw, rh) != (w, h);
        if offscreen {
            if g.fbo == 0 {
                gl::GenFramebuffers(1, &mut g.fbo);
                gl::GenTextures(1, &mut g.tex);
            }
            if g.fbo_size != (rw, rh) {
                gl::BindTexture(gl::TEXTURE_2D, g.tex);
                gl::TexImage2D(gl::TEXTURE_2D, 0, gl::RGBA8 as i32, rw, rh, 0, gl::RGBA, gl::UNSIGNED_BYTE, std::ptr::null());
                gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
                gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
                gl::BindFramebuffer(gl::FRAMEBUFFER, g.fbo);
                gl::FramebufferTexture2D(gl::FRAMEBUFFER, gl::COLOR_ATTACHMENT0, gl::TEXTURE_2D, g.tex, 0);
                g.fbo_size = (rw, rh);
            }
            gl::BindFramebuffer(gl::DRAW_FRAMEBUFFER, g.fbo);
        }
        gl::Viewport(0, 0, rw, rh);
        gl::UseProgram(g.program);
        let now = Instant::now();
        let t = now.duration_since(inner.start.get()).as_secs_f32();
        let dt = now.duration_since(inner.last.get()).as_secs_f32().min(1.0);
        inner.last.set(now);
        let l = g.loc;
        gl::Uniform3f(l[0], rw as f32, rh as f32, 1.0);
        gl::Uniform1f(l[1], t);
        gl::Uniform1f(l[2], dt);
        gl::Uniform1i(l[3], inner.frame.get());
        gl::Uniform4f(l[4], 0.0, 0.0, 0.0, 0.0);
        if let Ok(d) = glib::DateTime::now_local() {
            let secs = d.hour() as f32 * 3600.0 + d.minute() as f32 * 60.0 + d.seconds() as f32;
            gl::Uniform4f(l[5], d.year() as f32, (d.month() - 1) as f32, d.day_of_month() as f32, secs);
        }
        gl::Uniform1f(l[6], 44100.0);
        gl::Uniform1f(l[7], inner.fps.get() as f32);
        for i in 0..4u32 {
            gl::ActiveTexture(gl::TEXTURE0 + i);
            gl::BindTexture(gl::TEXTURE_2D, g.black);
        }
        gl::BindVertexArray(g.vao);
        gl::DrawArrays(gl::TRIANGLES, 0, 3);
        if offscreen {
            gl::BindFramebuffer(gl::READ_FRAMEBUFFER, g.fbo);
            gl::BindFramebuffer(gl::DRAW_FRAMEBUFFER, target as u32);
            gl::BlitFramebuffer(0, 0, rw, rh, 0, 0, w, h, gl::COLOR_BUFFER_BIT, gl::LINEAR);
            gl::BindFramebuffer(gl::FRAMEBUFFER, target as u32);
        }
        inner.frame.set(inner.frame.get() + 1);
    }
}
