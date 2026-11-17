//! Estado del core y sus decisiones (plan-core.md §1).
//!
//! Puro: [`World::step`] recibe un evento (una tecla, el resultado de un
//! pedido a Windows) y devuelve efectos (pedidos nuevos, líneas de log). El
//! brain (`app/src/brain.rs`) ejecuta los efectos con `wm-platform` y le
//! devuelve los resultados como eventos. Así toda la lógica se prueba sin
//! Windows, con una plataforma de mentira.
//!
//! Hecho: la capa de apps (`Caps` + letra), la capa de ventana (mitades,
//! esquinas, empujar, C/F/G/V/Z), los escritorios 1-4 (hito 4) y las reglas
//! por app (v1.1, `world_rules.rs`).
//!
//! **Capa de ventana** (plan-core.md §3.1, versión 2): cada tecla actúa según
//! **dónde está la ventana** ([`crate::place::step`]), no según la tecla
//! anterior ni el tiempo. Dos o más teclas juntas forman un **gesto**: el
//! destino se calcula desde donde estaba antes de la primera
//! ([`crate::place::chord`]), así que el orden en que caen los dedos da
//! igual. Empujar al monitor vecino con una tecla sola espera 80 ms por si
//! llega una segunda tecla (sin eso, la ventana iría y volvería).
//!
//! **Colocar una ventana** (plan-core.md §2.5): leer la ventana al frente
//! (`Inspect`), pedir el rectángulo (`Place`) y verificar con plazos (+16,
//! +50, +120, +250, +600 ms). Si el marco quedó lejos y ya no cambia, una
//! segunda pasada con los márgenes nuevos; si sigue lejos (tamaño mínimo,
//! app que no responde), se acepta lo logrado. Una sola colocación en curso
//! por ventana: la última tecla gana. Sin plazos pendientes, el brain duerme.
//!
//! **Animación** (hito 1e, plan-core.md §3.4): si el destino tiene el mismo
//! tamaño que la ventana (mitad ↔ mitad, esquina ↔ esquina, empujar entre
//! monitores con la misma escala, centrar), en vez de `Place` sale `Fly`: el
//! carril `anim` mueve un fantasma y la ventana real va en un solo pedido. Al
//! responder (`Flown`) siguen los chequeos de siempre. Cualquier otro pedido
//! sobre una ventana en vuelo la baja antes (`Land`).

use std::collections::{HashMap, VecDeque};

use crate::apps::{AppBinding, Target, resolve};
use crate::keyboard::{HyperAction, Layer};
use crate::map::{Map, MapOut};
use crate::place::{
    Dir, MonitorObs, Move, anchor, centered, centered_percent, chord, chord_name, chord_pushes,
    edge_error, mirror, monitor_of, neighbor, outer_for, placement_name, slot_at, slot_rect, step,
};
use crate::{Placement, Rect};

/// Una ventana observada con `Inspect` (siempre viene de Windows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Obs {
    pub key: WinKey,
    pub app: String,
    pub title: String,
    /// Marco visible (`DWMWA_EXTENDED_FRAME_BOUNDS`).
    pub frame: Rect,
    /// Rectángulo de la ventana (`GetWindowRect`), con los bordes invisibles.
    pub outer: Rect,
    pub dpi: u32,
    pub maximized: bool,
    pub minimized: bool,
    /// Los monitores en el momento de la observación.
    pub monitors: Vec<MonitorObs>,
    /// Por qué no se puede animar esta ventana (`None`: se puede).
    pub no_fly: Option<NoFly>,
    /// v1.1: dónde estaba el puntero (para "abrir en la pantalla del
    /// puntero"; `None` si no se leyó).
    pub pointer: Option<(i32, i32)>,
}

/// Por qué una ventana no se anima (se mueve instantánea).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoFly {
    /// No está al frente: el vuelo la subiría en el orden Z (Z sobre otra).
    NotFront,
    /// La app no responde (`IsHungAppWindow`).
    Hung,
    /// Contenido protegido: la miniatura sale negra.
    Protected,
    /// El sistema pide reducir animaciones.
    ReducedMotion,
}

/// Cómo terminó un vuelo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlyReport {
    /// La ventana real llegó al destino antes de quitar el fantasma.
    pub arrived: bool,
    pub flight_ms: u32,
}

/// Un vuelo que no se pudo hacer: la ventana se coloca sin animación.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlyFail {
    /// `true`: no se anima más en esta sesión (carril trabado, sin cloak).
    pub fatal: bool,
    pub why: String,
}

/// Recorrido mínimo para animar: menos se ve como un salto igual.
const FLY_MIN_PX: i32 = 64;

/// Cuánto se mueve el borde que más se mueve (al cambiar de tamaño, un borde
/// puede quedar quieto y otro recorrer media pantalla).
fn travel(from: Rect, to: Rect) -> i32 {
    [to.left - from.left, to.top - from.top, to.right - from.right, to.bottom - from.bottom]
        .into_iter()
        .map(i32::abs)
        .max()
        .unwrap_or(0)
}

/// Duración de un vuelo: más largo cuanto más lejos, entre 110 y 180 ms
/// (validados en el spike 6: 150 y 200 ms para 800 px, §13.8).
pub fn fly_ms(from: Rect, to: Rect) -> u32 {
    (100.0 + f64::from(travel(from, to)) / 16.0).clamp(110.0, 180.0) as u32
}

/// Chequeos después de pedir una ubicación, en ms desde el pedido.
const CHECKS_MS: [u64; 5] = [16, 50, 120, 250, 600];

/// Espera de una tecla sola que empuja al monitor vecino o sale de una
/// esquina, por si es la primera de un acorde. No cambia ningún resultado:
/// solo evita ver un paso intermedio. Con 50 ms, en la prueba del usuario
/// (16/11/2026) ~1 acorde de cada 10 llegaba tarde y pasaba por la mitad.
const CHORD_WAIT_US: u64 = 80_000;

/// Error aceptado por borde.
const TOLERANCE_PX: i32 = 1;

/// Entradas del historial para deshacer (Z).
const JOURNAL_MAX: usize = 32;

/// Una entrada del historial: cómo estaba una ventana antes de un movimiento
/// nuestro (plan-core.md §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    win: WinKey,
    frame: Rect,
    maximized: bool,
    /// Gesto que la anotó: un acorde (varias teclas) es una sola entrada.
    tag: u64,
    /// `Some(n)`: un "llevar" desde el escritorio n (hito 4); deshacerla la
    /// devuelve ahí (`frame` y `maximized` no se usan).
    desk: Option<u8>,
    /// v1.2: con `desk`, el usuario fue con la ventana ("escritorio al
    /// abrir"): deshacer vuelve con ella al escritorio de antes.
    went: bool,
}

/// Los pasos de centrar (C): primero sin cambiar el tamaño; repetir alterna
/// 60 % → 80 % → tamaño original.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CenterStep {
    Original,
    P60,
    P80,
}

/// G/V (hito 1f): cuánto cambia cada tecla y el mínimo, en porcentaje del
/// área de trabajo.
const SIZE_STEP_PCT: i32 = 10;
const SIZE_MIN_PCT: i32 = 20;

/// El ancla de G/V de una ventana, válida mientras siga donde la dejó: el
/// lugar contra cuyo borde exterior crece o se achica, y qué ejes cambian.
#[derive(Debug, Clone, Copy)]
struct Sizing {
    anchor: Rect,
    x: bool,
    y: bool,
    target: Rect,
}

/// El ciclo de C de una ventana, válido mientras siga donde la dejó.
#[derive(Debug, Clone, Copy)]
struct Centering {
    original: (i32, i32),
    step: CenterStep,
    target: Rect,
}

/// Una colocación en curso.
#[derive(Debug, Clone)]
struct Job {
    id: u64,
    what: String,
    target: Rect,
    key_us: u64,
    pass: u8,
    /// Cuándo se pidió la ubicación de esta pasada.
    placed_us: u64,
    /// Próximo chequeo (índice en `CHECKS_MS`).
    check: usize,
    /// Esperando un `Inspect` de verificación.
    inspecting: bool,
    last_frame: Option<Rect>,
    /// Marco al pedir esta pasada: mientras siga igual, la app todavía no
    /// aplicó el pedido (no es "quedó quieta en otro lugar").
    from_frame: Rect,
    /// DPI del monitor de destino: al cruzar a uno con otra escala, la
    /// ventana tiene que informarlo antes de juzgar dónde quedó.
    want_dpi: u32,
    /// Adónde va (para que la tecla siguiente cuente desde ahí).
    spot: Spot,
    /// Ya se hizo la pasada de anclaje (tamaño mínimo o fijo).
    anchored: bool,
    /// En vuelo: el carril `anim` responde con `Flown`; hasta entonces no
    /// hay chequeos.
    flying: bool,
    /// Lo que duró el vuelo, para el log.
    flight_ms: Option<u32>,
}

/// Dónde está (o va) una ventana: un monitor y una ubicación en él (`Full`
/// = libre o maximizada).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Spot {
    monitor: MonitorObs,
    placement: Placement,
}

/// Las teclas de un gesto, decididas **al llegar cada tecla**, en el mismo
/// orden que los cambios de capa (soltar el pulgar o Caps termina el gesto).
/// Decidirlo recién al volver el `Inspect` de la tecla tenía una carrera: en
/// una máquina lenta, soltar Alt se procesaba entre la respuesta de la
/// primera tecla y la de la segunda, y la segunda se tomaba como sola.
#[derive(Debug, Clone, Copy)]
struct Keys {
    id: u64,
    seen: u8,
}

/// Dos o más teclas de dirección juntas sobre la misma ventana.
#[derive(Debug, Clone, Copy)]
struct Gesture {
    /// El [`Keys::id`] del gesto.
    id: u64,
    win: WinKey,
    /// Estaba maximizada antes de la primera tecla (para deshacer).
    origin_max: bool,
    /// Dónde estaba antes de la primera tecla.
    origin: Spot,
    origin_rect: Rect,
    /// Teclas del gesto (`HELD_*`).
    seen: u8,
    /// Cuándo se apretó la primera tecla (para ver si la segunda llegó tarde).
    key_us: u64,
}

/// Una tecla sola demorada por si llega una segunda tecla: un empuje al
/// monitor vecino (sin esperar, la ventana iría y volvería) o, desde una
/// esquina, la mitad (sin esperar, la mitad cambia el tamaño y el acorde de
/// esquina a esquina ya no se podría animar).
#[derive(Debug, Clone)]
struct Deferred {
    due_us: u64,
    /// Para el historial: cómo estaba antes.
    before: Entry,
    obs: Obs,
    what: String,
    key_us: u64,
    to: DeferredTo,
}

#[derive(Debug, Clone)]
enum DeferredTo {
    /// Una ubicación en el mismo monitor.
    Here(Spot),
    /// `Some`: al monitor vecino; `None`: no hay vecino (solo se informa).
    /// `fix`: sin vecino y a 1 px de su lugar, dejarla exacta ahí.
    Push { to: Option<Spot>, dir: Dir, from: Placement, fix: Option<Spot> },
}

/// Identificador de un pedido del modelo a la plataforma.
pub type Req = u64;

/// Una ventana: su HWND y el proceso dueño. El proceso evita actuar sobre un
/// HWND que Windows reutilizó para otra ventana (plan-core.md §1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct WinKey {
    pub hwnd: isize,
    pub pid: u32,
}

/// Una ventana de usuario, como la ve un `Scan`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WinInfo {
    pub key: WinKey,
    /// Ejecutable sin extensión, en minúsculas.
    pub app: String,
    pub title: String,
    /// Escritorio anotado en la ventana (`wm.desk`).
    pub desk: Option<u8>,
    /// Tiene nuestra marca de escondida.
    pub marked: bool,
    /// No se ve: con cloak o minimizada.
    pub concealed: bool,
    pub minimized: bool,
    /// El ejecutable de su familia, si es otro (plan-apps.md §2).
    pub family: Option<String>,
    /// AUMID del proceso (apps MSIX y de la Store).
    pub aumid: Option<String>,
    /// Ventana principal: se puede redimensionar y maximizar (no es una
    /// pantalla de carga ni un diálogo). v1.1: solo a estas se les aplica la
    /// regla "al abrir".
    pub main: bool,
}

impl WinInfo {
    /// Quién es su app, para `apps::resolve`.
    pub fn app_id(&self) -> crate::apps::AppId<'_> {
        crate::apps::AppId {
            exe: &self.app,
            family: self.family.as_deref(),
            aumid: self.aumid.as_deref(),
        }
    }
}

/// Resultado de traer una ventana al frente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusReport {
    pub ok: bool,
    pub neutral_key: bool,
    pub elapsed_us: u64,
}

/// Lo que le llega al modelo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Una acción de Hyper. `age_ms`: cuánto tardó en llegar desde la tecla.
    Action {
        action: HyperAction,
        held: u8,
        age_ms: u32,
    },
    /// Cambió la capa del teclado.
    Layer(Layer),
    /// El estado del teclado se reinició (callback lento, hook reinstalado):
    /// la capa vuelve a `Off` sin aviso del hook.
    KeyboardReset,
    /// Resultado de un `Scan`: ventanas en orden Z (la primera, arriba) y la
    /// que tiene el foco.
    Windows {
        req: Req,
        result: Result<(Vec<WinInfo>, Option<WinKey>), String>,
    },
    Focused {
        req: Req,
        result: Result<FocusReport, String>,
    },
    Launched {
        req: Req,
        result: Result<(), String>,
    },
    /// Asignaciones nuevas (al arrancar o al recargar `apps.txt`).
    Bindings(Vec<AppBinding>),
    Inspected {
        req: Req,
        result: Result<Obs, String>,
    },
    Placed {
        req: Req,
        result: Result<(), String>,
    },
    /// Terminó un vuelo (o no se pudo hacer).
    Flown {
        req: Req,
        result: Result<FlyReport, FlyFail>,
    },
    /// Se puede animar (el carril `anim` está listo) o ya no.
    FlyEnabled(bool),
    /// Se puede mostrar el mapa (el hilo `ui` está listo) o ya no.
    MapEnabled(bool),
    /// Una orden del menú de la bandeja (o de `wm quit|pause|resume|undo`).
    Command(Command),
    /// Venció el plazo de [`World::deadline`].
    Tick,
    /// Hito 4: el core terminó de restaurar al arrancar. `recover`: lo
    /// relanzó el guardián tras una caída (D4).
    Boot {
        recover: bool,
    },
    /// Resultado de `SetDesks`: las ventanas que no se pudieron anotar.
    DesksSet {
        req: Req,
        failed: Vec<WinKey>,
    },
    /// Resultado de `Show` (`Ok`: las que fallaron).
    Shown {
        req: Req,
        result: Result<Vec<WinKey>, String>,
    },
    /// Resultado de `Hide` (`Ok`: las que fallaron).
    Hidden {
        req: Req,
        result: Result<Vec<WinKey>, String>,
    },
    /// Hito 4b.2: esta ventana pasó al frente (ya traducida al dueño raíz).
    Foreground(WinKey),
    /// Hito 4b.2: a esta ventana le quitaron el cloak.
    Uncloaked(WinKey),
    /// Hito 5: con `Caps` se apretó (`down`) o soltó un botón del mouse en
    /// `(x, y)`.
    Pointer {
        button: crate::mouse::Button,
        down: bool,
        x: i32,
        y: i32,
    },
    /// Hito 5: la ventana bajo el cursor (respuesta de `InspectAt`).
    Grabbed {
        req: Req,
        result: Result<Grab, String>,
    },
    /// Opciones nuevas (`settings.txt`, al arrancar y al guardarse).
    Options(crate::options::Options),
    /// Hito 5: terminó un arrastre.
    DragDone {
        win: WinKey,
        result: Result<DragReport, String>,
    },
    /// Hito 4d: aviso de pantallas, reposo o sesión.
    Screen(crate::screens::ScreenEvent),
    /// Hito 4d: resultado de `Survey`.
    Surveyed {
        req: Req,
        result: Result<Survey, String>,
    },
    /// La fila de la lista en el punto del clic (`None`: afuera).
    ListHit {
        req: Req,
        row: Option<usize>,
    },
    /// Hito 4d: resultado de `Rescue` (las que no se pudieron mover).
    Rescued {
        req: Req,
        failed: Vec<(WinKey, String)>,
    },
}

/// La ventana agarrada con `Caps` + clic (hito 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grab {
    pub obs: Obs,
    /// Maximizada: su tamaño normal (exterior).
    pub restore: Option<(i32, i32)>,
    /// Tiene borde para cambiar el tamaño.
    pub resizable: bool,
}

/// Cómo terminó un arrastre (hito 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DragReport {
    /// Se movió de verdad (no fue un clic quieto).
    pub moved: bool,
    /// Marco visible al terminar.
    pub frame: Option<Rect>,
    pub ms: u32,
}

/// Lo que el modelo le pide a la plataforma.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Scan {
        req: Req,
    },
    Focus {
        req: Req,
        win: WinKey,
    },
    Launch {
        req: Req,
        path: String,
        /// Sus ejecutables.
        names: Vec<String>,
        /// Abrirla solo si ninguno de esos ejecutables corre (durante la
        /// espera; si corre, responde [`LOADING`]).
        only_if_closed: bool,
    },
    /// Abrir de nuevo una app que corre sin ventanas (en la bandeja), desde
    /// el ejecutable de su proceso. Responde con `Launched`.
    Revive {
        req: Req,
        names: Vec<String>,
    },
    /// Observar una ventana (`None` = la que tiene el foco, si se puede
    /// acomodar).
    Inspect {
        req: Req,
        win: Option<WinKey>,
    },
    /// Pedir el rectángulo exterior (asíncrono, sin activar).
    Place {
        req: Req,
        win: WinKey,
        rect: Rect,
    },
    /// Mover con animación: fantasma de `from` a `to` (marcos visibles) en
    /// `ms`, y la ventana real con un solo pedido a `rect` (exterior). Si la
    /// ventana ya está en vuelo, el carril la redirige desde donde va.
    Fly {
        req: Req,
        win: WinKey,
        rect: Rect,
        from: Rect,
        to: Rect,
        ms: u32,
    },
    /// Terminar ya el vuelo de esta ventana (va a recibir otro pedido).
    Land {
        win: WinKey,
    },
    /// Mostrar el mapa de teclado con esta capa (plan-core.md §5). `views`:
    /// las dos capas (apps y ventana), con las apps abiertas ahora.
    ShowMap {
        layer: Layer,
        views: Vec<crate::mapview::MapView>,
    },
    /// Con el mapa visible se usó esta tecla: pintarla en `accent`.
    MapKey {
        letter: char,
    },
    /// El mapa ya visible: mostrar otra capa.
    MapLayer {
        layer: Layer,
    },
    HideMap,
    /// Reiniciar la máquina de teclado del hook (tope del mapa vencido).
    ResetKeyboard,
    /// Sacarla de maximizada (o minimizada) sin activarla.
    Normalize {
        win: WinKey,
    },
    /// Maximizarla, como el botón de la ventana.
    Maximize {
        win: WinKey,
    },
    Log(String),
    /// Pausar (todas las teclas pasan, `Caps` vuelve a ser Bloq Mayús) o
    /// reanudar el hook.
    SetPaused(bool),
    /// Terminar el core ("Salir" en la bandeja; `Caps` + Esc solo si
    /// [`World::set_esc_quits`]).
    Quit,
    /// Hito 4: anotar el escritorio en cada ventana (`wm.desk`).
    SetDesks {
        req: Req,
        items: Vec<(WinKey, u8)>,
    },
    /// Hito 4: mostrar estas ventanas (las escondimos nosotros).
    Show {
        req: Req,
        wins: Vec<WinKey>,
    },
    /// Hito 4: esconderlas (marca + cloak, o minimize si no hay cloak).
    Hide {
        req: Req,
        wins: Vec<WinKey>,
    },
    /// Hito 4: foco al escritorio de Windows (destino vacío). Responde con
    /// `Focused`.
    FocusShell {
        req: Req,
    },
    /// Hito 5: qué ventana hay en `(x, y)`. Responde con `Grabbed`.
    InspectAt {
        req: Req,
        x: i32,
        y: i32,
    },
    /// Hito 5: traerla al frente (sin esperar respuesta).
    Raise {
        win: WinKey,
    },
    /// Hito 5: seguir al cursor desde `grab` (el carril `wm-drag`).
    /// `restore`: maximizada, su tamaño normal.
    DragStart {
        win: WinKey,
        mode: crate::drag::Mode,
        grab: (i32, i32),
        restore: Option<(i32, i32)>,
    },
    /// Hito 5: se soltó el botón. Responde con `DragDone`.
    DragStop {
        win: WinKey,
    },
    /// Un aviso corto en pantalla (una tecla no hizo nada).
    Notice(String),
    /// "El puntero sigue al foco": llevar el puntero al centro de la ventana
    /// (si no está ya adentro).
    MovePointer {
        win: WinKey,
    },
    /// Hito 4d: monitores y ventanas (responde con `Surveyed`).
    Survey {
        req: Req,
    },
    /// Hito 4d: correr estas ventanas a la esquina nueva de su rectángulo
    /// exterior, sin cambiar el tamaño (responde con `Rescued`).
    Rescue {
        req: Req,
        moves: Vec<(WinKey, i32, i32)>,
    },
    /// Hito 4d: cambiaron los monitores (rearmar el mapa y los avisos).
    ScreensChanged,
    /// La lista de ventanas de una letra (plan-apps.md §8): mostrarla o
    /// actualizarla.
    ShowList {
        view: crate::listview::ListView,
    },
    HideList,
    /// ¿Qué fila de la lista hay en `(x, y)`? Responde con `ListHit`.
    ListHit {
        req: Req,
        x: i32,
        y: i32,
    },
    /// v1.2: escribir esta regla "al abrir" para la letra en `apps.txt`
    /// (el brain, en un hilo; `wm-watch` lo recarga).
    SaveRule {
        letter: u8,
        open: Option<crate::rules::OpenRule>,
    },
}

/// Antigüedad máxima de una acción para ejecutarla: una que llega tarde (el
/// programa estuvo trabado) ya no es lo que el usuario quiere ahora.
pub const MAX_ACTION_AGE_MS: u32 = 1000;

/// Cuántas veces se intentó traer una app al frente y cómo salió.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Activations {
    pub total: u32,
    pub ok: u32,
    pub neutral_key: u32,
}

#[derive(Debug, Clone)]
enum Pending {
    /// Enumerar ventanas para resolver `Caps` + letra.
    Scan {
        letter: u8,
        key_us: u64,
    },
    Focus {
        letter: u8,
        app: String,
        title: String,
        key_us: u64,
        win: WinKey,
    },
    Launch {
        letter: u8,
        app: String,
        path: String,
        /// Pedido durante la espera: se abre solo si la app ya no corre.
        retry: bool,
    },
    /// Abrir de nuevo una app viva sin ventanas (en la bandeja).
    Revive {
        letter: u8,
        app: String,
    },
    /// Después de abrir una app: ¿ya tiene ventana? (para enfocarla)
    FollowScan,
    /// Capa de ventana: esperando la ventana al frente.
    Arrange {
        dir: Dir,
        key_us: u64,
        what: String,
        /// Gesto al que pertenece la tecla y sus teclas hasta ella.
        gesture: u64,
        seen: u8,
        /// `true`: continúa un gesto (es la segunda tecla o más).
        continues: bool,
    },
    /// F: maximizar o volver.
    Full {
        key_us: u64,
        what: String,
    },
    /// G/V: agrandar (`grow`) o achicar.
    Resize {
        key_us: u64,
        what: String,
        grow: bool,
    },
    /// C: centrar.
    Center {
        key_us: u64,
        what: String,
    },
    /// Z: deshacer esta entrada (y, si la ventana ya no existe, la anterior).
    Undo {
        key_us: u64,
        entry: Entry,
    },
    /// Verificar una colocación.
    Verify {
        win: WinKey,
        job: u64,
    },
    /// Las ventanas abiertas, para el contenido del mapa.
    MapScan,
    /// Respuesta del pedido de ubicación.
    PlaceCmd {
        win: WinKey,
        job: u64,
    },
    /// Respuesta de un vuelo; `rect`: el pedido sin animación si falla.
    FlyCmd {
        win: WinKey,
        job: u64,
        rect: Rect,
    },
    /// Hito 5: la ventana bajo el cursor.
    Grab,
    /// Hito 4: pasos de la transacción de escritorios.
    DeskScan,
    DeskSet,
    DeskShow,
    DeskFocus,
    DeskHide,
    /// Hito 4d.
    Survey,
    Rescue,
    /// v1.1 (d): cambió el espacio; lo encajado con `old_gap` se reacomoda.
    Regap {
        old_gap: u8,
    },
    /// Un clic sobre la lista de ventanas.
    ListHit,
    /// v1.1: observar una ventana nueva con regla "al abrir".
    Arrive {
        win: WinKey,
    },
    /// v1.2: "Abrir siempre así": cómo está la ventana.
    Keep {
        letter: u8,
    },
}

/// El estado del core.
#[derive(Debug, Default)]
pub struct World {
    bindings: Vec<AppBinding>,
    layer: Layer,
    next_req: Req,
    pending: HashMap<Req, Pending>,
    activations: Activations,
    jobs: HashMap<WinKey, Job>,
    next_job: u64,
    keys: Option<Keys>,
    next_gesture: u64,
    journal: VecDeque<Entry>,
    /// Pausado desde la bandeja: no actúa sobre teclas ni capas.
    paused: bool,
    /// `Caps` + Esc sale del programa. Hasta la bandeja (hito 3b) era la
    /// única salida; ahora solo en modo prueba: un toque accidental cerraba
    /// todo en silencio (Fable, hito 3).
    esc_quits: bool,
    /// Entrada a anotar si el próximo `go` mueve la ventana.
    record: Option<Entry>,
    centers: HashMap<WinKey, Centering>,
    sizing: HashMap<WinKey, Sizing>,
    /// El `go` en curso viene de G/V (conserva el ancla).
    sizing_move: bool,
    /// El `go` en curso pide el lugar exacto (sin la tolerancia de 1 px):
    /// ajuste sin vecino o reencaje tras un cambio de pantallas.
    exact: bool,
    gesture: Option<Gesture>,
    deferred: Option<Deferred>,
    /// Dónde dejamos cada ventana y qué marco logró: mientras siga igual, la
    /// próxima tecla cuenta desde esa ubicación aunque la app no haya podido
    /// achicarse del todo (tamaño mínimo).
    placed: HashMap<WinKey, (Spot, Rect)>,
    /// Se puede animar (lo habilita el brain cuando el carril está listo).
    fly: bool,
    /// El mapa de teclado (hito 2a).
    map: Map,
    /// Pedido de ventanas en curso para mostrar el mapa.
    map_scan: Option<Req>,
    /// Ya se mandó `ShowMap` (y todavía no `HideMap`).
    map_shown: bool,
    /// Las apps abiertas cuando se armó el mapa.
    map_open: Vec<String>,
    /// Las ventanas (orden Z) y la del frente cuando se armó el mapa: para
    /// mostrar a qué ventana va cada letra, y actualizarlo en cada toque.
    map_wins: Vec<WinInfo>,
    map_fg: Option<WinKey>,
    /// Lo que se sabe del tamaño mínimo o máximo de cada ventana (de las
    /// veces que no tomó el tamaño pedido): un cambio de tamaño que no
    /// respeta esos límites va sin animación.
    limits: HashMap<WinKey, Limits>,
    /// Escritorios 1-4 (hito 4).
    desks: crate::desks::Desks,
    /// Transacción de escritorios en curso.
    sync: Option<world_desks::DeskSync>,
    /// La próxima, si llegó otra mientras tanto (gana la última).
    queued: Option<(crate::desks::Goal, u64)>,
    /// Reconciliar a esta hora (un destape inesperado, hito 4b.2).
    verify_due: Option<u64>,
    /// Un botón del mouse tragado con `Caps` (hito 5).
    pointer: Option<world_drag::Pointer>,
    /// El arrastre en curso: ventana, cómo estaba antes y desde cuándo.
    dragging: Option<world_drag::Dragging>,
    /// Z llegó mientras un arrastre terminaba: se hace al terminar.
    undo_after_drag: Option<u64>,
    /// Opciones de `settings.txt`.
    options: crate::options::Options,
    /// Pantallas, reposo y bloqueo (hito 4d).
    screens: world_screens::Screens,
    /// "El Alt+Tab de la letra" (plan-apps.md §5): la lista de la pulsación
    /// en curso; se olvida al soltar `Caps`.
    cycle: Option<Cycle>,
    /// Apps abiertas hace poco (nombre → hasta cuándo, µs): otra pulsación
    /// solo avisa "Abriendo…" (AutoCAD y Revit tardan; no abrir dos).
    launching: HashMap<String, u64>,
    /// Después de abrir: buscar su ventana y enfocarla.
    follow: Option<Follow>,
    /// La ventana al frente cuando se pidió abrir (pasa a `Follow`).
    follow_fg: Option<WinKey>,
    /// El `Focus` del seguimiento en curso: si falla, se sigue esperando.
    follow_focus: Option<(Req, Follow)>,
    /// v1.1: ventanas nuevas esperando su regla "al abrir".
    arrivals: HashMap<WinKey, world_rules::Arrival>,
    /// v1.1: ventanas nuevas con regla que aparecieron detrás (IntelliJ:
    /// Java la muestra y la activa un momento después). Si pasan al frente
    /// pronto, la regla corre: (regla, cuándo se vieron).
    behind: HashMap<WinKey, (world_rules::Want, u64)>,
    /// v1.1: una ventana pasó al frente pero el `Scan` no la vio (Java la
    /// activa antes de terminar de mostrarla): se la vuelve a buscar.
    unseen: Option<world_rules::Unseen>,
    /// v1.2: la regla que "Abrir siempre así" mandó a guardar; al volver
    /// en `apps.txt`, se confirma con un aviso.
    keeping: Option<(u8, crate::rules::OpenRule)>,
    /// v1.2: cuándo pidió el usuario el último cambio de escritorio (µs):
    /// después de abrirse una ventana, gana sobre "escritorio: N".
    user_desk_us: u64,
}

/// La lista de una letra mientras `Caps` sigue abajo.
#[derive(Debug, Clone)]
struct Cycle {
    letter: u8,
    order: Vec<WinKey>,
    pos: usize,
    /// Modo lista (3 o más ventanas, desde el segundo toque): los toques
    /// mueven la selección sin enfocar; soltar `Caps` va a la elegida.
    list: bool,
    /// Mostrarla a esta hora (150 ms después del segundo toque).
    list_due: Option<u64>,
    list_shown: bool,
    /// Desde cuándo está abierta (tope de 30 s).
    list_since: u64,
    /// La ventana que quedó al frente con el primer toque.
    front: Option<WinKey>,
    /// Las ventanas del último `Scan` (títulos, escritorios) y la del frente.
    snapshot: Vec<WinInfo>,
    fg: Option<WinKey>,
}

/// Lista recién pedida: se muestra si `Caps` sigue abajo a los 150 ms.
const LIST_DELAY_US: u64 = 150_000;
/// Una lista abierta más de esto se cierra sola.
const LIST_MAX_US: u64 = 30_000_000;
const VK_UP: u8 = 0x26;
const VK_DOWN: u8 = 0x28;

/// Seguimiento de una app recién abierta.
#[derive(Debug, Clone)]
struct Follow {
    letter: u8,
    launched_us: u64,
    /// Próximo chequeo (índice en `FOLLOW_MS`).
    step: usize,
    /// La ventana al frente al abrirla: si cambió, el usuario ya hizo otra
    /// cosa y no se le roba el foco.
    fg: Option<WinKey>,
    scanning: bool,
    /// Ventanas que no se dejaron traer al frente (la pantalla de carga de
    /// IntelliJ): se siguen esperando las demás.
    skip: Vec<WinKey>,
}

/// Cuándo mirar si la app recién abierta ya tiene ventana (las pesadas,
/// como AutoCAD o Revit, tardan cerca de un minuto).
const FOLLOW_MS: [u64; 8] = [300, 800, 1500, 3000, 6000, 12_000, 30_000, 60_000];
/// Respuesta de `Launch` con `only_if_closed` cuando la app sigue corriendo
/// (cargando): no es un error, se sigue esperando su ventana.
pub const LOADING: &str = "cargando";
/// Cuánto dura la protección contra abrir dos veces: hasta que aparece su
/// ventana o 90 s (AutoCAD y Revit tardan). Solo la que abrimos nosotros:
/// una app que el usuario cerró se abre de nuevo al toque (antes se miraba
/// la edad del proceso, y una IntelliJ recién cerrada quedaba "abriendo" sin
/// abrirse; prueba del 17/11/2026).
const LAUNCH_GUARD_US: u64 = 90_000_000;

/// Tamaños que una ventana no aceptó. Antes se marcaba la ventana entera y
/// después de un solo V hasta el límite (Brave no baja de 502 px) ningún
/// cambio de tamaño se animaba más (prueba del usuario, 16/11/2026).
#[derive(Debug, Clone, Copy, Default)]
struct Limits {
    min_w: Option<i32>,
    min_h: Option<i32>,
    max_w: Option<i32>,
    max_h: Option<i32>,
}

impl Limits {
    /// Pidió `asked` y quedó en `got`: en cada eje, si quedó más grande, ese
    /// es su mínimo; si quedó más chica, su máximo.
    fn learn(&mut self, got: Rect, asked: Rect) {
        let axis = |got: i32, asked: i32, min: &mut Option<i32>, max: &mut Option<i32>| {
            if got > asked + TOLERANCE_PX {
                *min = Some(got);
            } else if got < asked - TOLERANCE_PX {
                *max = Some(got);
            }
        };
        axis(got.width(), asked.width(), &mut self.min_w, &mut self.max_w);
        axis(got.height(), asked.height(), &mut self.min_h, &mut self.max_h);
    }

    /// ¿Puede tomar el tamaño de `r`, por lo que se sabe?
    fn allows(&self, r: Rect) -> bool {
        let ok = |v: i32, min: Option<i32>, max: Option<i32>| {
            min.is_none_or(|m| v >= m - TOLERANCE_PX) && max.is_none_or(|m| v <= m + TOLERANCE_PX)
        };
        ok(r.width(), self.min_w, self.max_w) && ok(r.height(), self.min_h, self.max_h)
    }
}

impl World {
    pub fn new(bindings: Vec<AppBinding>) -> Self {
        Self { bindings, ..Self::default() }
    }

    /// ¿Pausado desde la bandeja?
    pub fn paused(&self) -> bool {
        self.paused
    }

    /// ¿Hay algo para deshacer? (el ítem del menú va gris si no)
    pub fn can_undo(&self) -> bool {
        !self.journal.is_empty()
    }

    /// Que `Caps` + Esc salga del programa (modo prueba y `ci-probe`).
    pub fn set_esc_quits(&mut self, on: bool) {
        self.esc_quits = on;
    }

    pub fn layer(&self) -> Layer {
        self.layer
    }

    pub fn activations(&self) -> Activations {
        self.activations
    }

    pub fn bindings(&self) -> &[AppBinding] {
        &self.bindings
    }

    /// Pedidos todavía sin respuesta.
    pub fn in_flight(&self) -> usize {
        self.pending.len()
    }

    /// Colocaciones en curso.
    pub fn placing(&self) -> usize {
        self.jobs.len()
    }

    /// La ventana está en vuelo (sus avisos de cloak son nuestros).
    pub fn flying(&self, win: WinKey) -> bool {
        self.jobs.get(&win).is_some_and(|j| j.flying)
    }

    /// Cuándo hay que despertar al modelo con [`Event::Tick`] (µs del mismo
    /// reloj que `step`). `None`: nada pendiente, el brain duerme.
    pub fn deadline(&self) -> Option<u64> {
        // Un vuelo no tiene plazos del modelo: lo vigila el brain.
        let jobs = self.jobs.values().filter(|j| !j.inspecting && !j.flying).map(next_check_us);
        jobs.chain(self.deferred.as_ref().map(|d| d.due_us))
            .chain(self.map.deadline())
            .chain(self.verify_due)
            .chain(self.pointer_deadline())
            .chain(self.screens.deadline_us())
            .chain(self.follow_deadline())
            .chain(self.list_deadline())
            .chain(self.arrival_deadline())
            .chain(self.unseen_deadline())
            .min()
    }

    /// El mapa de teclado se ve ahora.
    pub fn map_visible(&self) -> bool {
        self.map.visible()
    }

    /// Traduce lo que pide el mapa. Mostrarlo necesita las ventanas abiertas
    /// (las apps automáticas por inicial): se pide un `Scan` y `ShowMap` sale
    /// con la respuesta (~2 ms). Si se oculta antes, no sale nada.
    fn map_effects(&mut self, outs: Vec<MapOut>, out: &mut Vec<Effect>) {
        for o in outs {
            match o {
                MapOut::Show(_) => {
                    let req = self.req();
                    self.pending.insert(req, Pending::MapScan);
                    self.map_scan = Some(req);
                    out.push(Effect::Scan { req });
                }
                MapOut::Layer(layer) => {
                    if self.map_shown {
                        out.push(Effect::MapLayer { layer });
                    }
                }
                MapOut::Hide => {
                    if let Some(req) = self.map_scan.take() {
                        self.pending.remove(&req);
                    }
                    if std::mem::take(&mut self.map_shown) {
                        out.push(Effect::HideMap);
                    }
                }
                MapOut::ResetKeyboard => {
                    // El hook vuelve a Off sin avisar: lo mismo acá.
                    self.layer = Layer::Off;
                    self.keys = None;
                    out.push(Effect::ResetKeyboard);
                }
            }
        }
    }

    /// Llegaron las ventanas para el mapa: mostrarlo (si sigue visible).
    fn show_map(&mut self, windows: &[WinInfo], out: &mut Vec<Effect>) {
        self.map_scan = None;
        if !self.map.visible() {
            return;
        }
        self.map_open = windows
            .iter()
            .flat_map(|w| w.family.iter().cloned().chain(std::iter::once(w.app.clone())))
            .collect();
        self.map_shown = true;
        self.send_map(out);
    }

    /// Manda el mapa con las apps abiertas de la última vez y los escritorios
    /// de ahora (también al terminar un cambio de escritorio con el mapa
    /// visible).
    fn send_map(&mut self, out: &mut Vec<Effect>) {
        let open: Vec<&str> = self.map_open.iter().map(String::as_str).collect();
        let desks = crate::mapview::DeskSummary {
            active: self.desks.active(),
            counts: self.desks.counts(),
        };
        let targets: Vec<(u8, String, usize)> =
            (b'A'..=b'Z').filter_map(|l| self.predict(l).map(|(a, n)| (l, a, n))).collect();
        let views = vec![
            crate::mapview::map_view_targets(Layer::Apps, &self.bindings, &open, desks, &targets),
            crate::mapview::map_view_targets(Layer::Window, &self.bindings, &open, desks, &targets),
        ];
        out.push(Effect::ShowMap { layer: self.layer, views });
    }

    /// A qué app va el próximo toque de `letter` (lo mismo que `pick`, sin
    /// cambiar nada), con las ventanas de cuando se armó el mapa.
    fn predict(&self, letter: u8) -> Option<(String, usize)> {
        let list: Vec<(WinKey, crate::apps::AppId)> =
            self.map_wins.iter().map(|w| (w.key, w.app_id())).collect();
        let Target::Focus { windows: order, .. } = resolve(letter, &self.bindings, &list) else {
            return None;
        };
        let fg = self.map_fg;
        let next = match self.cycle.as_ref().filter(|c| c.letter == letter) {
            Some(c) => {
                let alive: Vec<WinKey> =
                    c.order.iter().copied().filter(|w| order.contains(w)).collect();
                let cur = c.order.get(c.pos).copied();
                let i = cur.and_then(|w| alive.iter().position(|&x| x == w)).unwrap_or(0);
                alive.get((i + 1) % alive.len().max(1)).copied()
            }
            None => match fg.filter(|f| order.contains(f)) {
                Some(f) => order.iter().copied().find(|&w| w != f).or(Some(f)),
                None => order.first().copied(),
            },
        }?;
        let n = order.len();
        self.map_wins.iter().find(|w| w.key == next).map(|w| (w.app_id().name().to_owned(), n))
    }

    /// Una ventana pasó al frente con el mapa visible: va arriba en el orden
    /// guardado y el mapa se actualiza (el próximo toque va a otra).
    fn map_focused(&mut self, win: WinKey, out: &mut Vec<Effect>) {
        if let Some(i) = self.map_wins.iter().position(|w| w.key == win) {
            let w = self.map_wins.remove(i);
            self.map_wins.insert(0, w);
        }
        self.map_fg = Some(win);
        if self.map_shown {
            self.send_map(out);
        }
    }

    fn req(&mut self) -> Req {
        self.next_req += 1;
        self.next_req
    }

    /// Procesa un evento. `now_us`: reloj monótono en microsegundos (el mismo
    /// para todos los eventos), para medir desde la tecla.
    pub fn step(&mut self, event: Event, now_us: u64) -> Vec<Effect> {
        let mut out = self.step_inner(event, now_us);
        // Avisos en pantalla: cuando una tecla no hizo nada, además del log.
        if self.options.notices
            && let Some(text) = out.iter().rev().find_map(|e| match e {
                Effect::Log(l) => crate::notices::notice_for(l),
                _ => None,
            })
        {
            out.push(Effect::Notice(text));
        }
        out
    }

    fn step_inner(&mut self, event: Event, now_us: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        match event {
            // En pausa, el hook deja pasar todo; si igual llega algo (en
            // vuelo al pausar), se ignora.
            Event::Action { .. } | Event::Layer(_) if self.paused => {}
            Event::Action { action, held, age_ms } => {
                self.screens_unblock(now_us, &mut out);
                if self.cycle.as_ref().is_some_and(|c| c.list) {
                    match action {
                        HyperAction::Escape => {
                            // Cerrar y quedarse en la ventana que se ve.
                            self.drop_cycle(&mut out);
                            return out;
                        }
                        HyperAction::Other { vk: VK_UP | VK_DOWN, .. } => {
                            let delta = if matches!(action, HyperAction::Other { vk: VK_UP, .. }) {
                                -1
                            } else {
                                1
                            };
                            self.list_move(delta, None, now_us, &mut out);
                            return out;
                        }
                        _ => {}
                    }
                }
                // Cualquier acción (también una descartada) suprime el mapa si
                // todavía no se ve; Esc con el mapa visible solo lo cierra.
                let mut m = Vec::new();
                let consumed = self.map.on_action(action == HyperAction::Escape, now_us, &mut m);
                self.map_effects(m, &mut out);
                if consumed {
                    return out;
                }
                if self.map_shown {
                    match action {
                        HyperAction::Letter { letter, .. } => {
                            out.push(Effect::MapKey { letter: letter as char })
                        }
                        HyperAction::Digit { digit: d @ 1..=4, .. } => {
                            out.push(Effect::MapKey { letter: char::from(b'0' + d) })
                        }
                        _ => {}
                    }
                }
                if self.pointer.is_some() {
                    let what = describe(action);
                    out.push(Effect::Log(format!("{what}: ignorada durante un arrastre.")));
                } else if age_ms > MAX_ACTION_AGE_MS {
                    let what = describe(action);
                    out.push(Effect::Log(format!(
                        "Acción descartada: llegó {age_ms} ms tarde ({what})."
                    )));
                } else {
                    self.action(action, held, age_ms, now_us, &mut out);
                }
            }
            Event::Layer(layer) => {
                let mut m = Vec::new();
                self.map.on_layer(self.layer, layer, now_us, &mut m);
                self.map_effects(m, &mut out);
                self.layer = layer;
                if layer != Layer::Window {
                    self.keys = None; // las teclas siguientes empiezan otro gesto
                }
                if layer == Layer::Off {
                    // Soltar Caps confirma: con lista, ir a la elegida.
                    self.commit_list(now_us, &mut out);
                }
            }
            Event::KeyboardReset => {
                self.drop_cycle(&mut out);
                self.pointer_reset(&mut out);
                self.layer = Layer::Off;
                self.keys = None;
                let mut m = Vec::new();
                self.map.reset(&mut m);
                self.map_effects(m, &mut out);
            }
            Event::MapEnabled(on) => {
                if !on {
                    self.drop_cycle(&mut out);
                }
                let mut m = Vec::new();
                self.map.set_enabled(on, &mut m);
                self.map_effects(m, &mut out);
            }
            Event::Bindings(b) => {
                self.bindings = b;
                self.kept(&mut out);
            }
            Event::Windows { req, result } => {
                if let Some(Pending::FollowScan) = self.pending.get(&req) {
                    self.pending.remove(&req);
                    if let Ok((windows, fg)) = result {
                        // Suele ser el primero en ver la ventana nueva.
                        self.adopt_scan(&windows, fg, now_us, &mut out);
                        self.follow_scanned(&windows, fg, now_us, &mut out);
                    } else if let Some(f) = self.follow.as_mut() {
                        f.scanning = false;
                    }
                    return out;
                }
                if let Some(Pending::DeskScan) = self.pending.get(&req) {
                    self.pending.remove(&req);
                    self.sync_scanned(result, now_us, &mut out);
                    return out;
                }
                if let Some(Pending::MapScan) = self.pending.get(&req) {
                    self.pending.remove(&req);
                    let (windows, fg) = result.unwrap_or_default();
                    self.adopt_scan(&windows, fg, now_us, &mut out);
                    self.map_fg = fg;
                    self.map_wins = windows.clone();
                    self.show_map(&windows, &mut out);
                    return out;
                }
                let Some(Pending::Scan { letter, key_us }) = self.pending.remove(&req) else {
                    return out;
                };
                match result {
                    Ok((windows, fg)) => {
                        self.adopt_scan(&windows, fg, now_us, &mut out);
                        self.go_to_app(letter, key_us, &windows, fg, now_us, &mut out)
                    }
                    Err(e) => out.push(Effect::Log(format!("Caps + {}: {e}", letter as char))),
                }
            }
            Event::Focused { req, result } => {
                if let Some(Pending::DeskFocus) = self.pending.get(&req) {
                    self.pending.remove(&req);
                    self.sync_focused(result, now_us, &mut out);
                    return out;
                }
                let Some(Pending::Focus { letter, app, title, key_us, win }) =
                    self.pending.remove(&req)
                else {
                    return out;
                };
                // Si ya estaba al frente, Windows no avisa `FOREGROUND`: una
                // nueva que esperaba detrás arranca su regla igual.
                if result.as_ref().is_ok_and(|f| f.ok) {
                    self.came_forward(win, now_us, &mut out);
                }
                if let Some((r, next)) = self.follow_focus.take()
                    && r == req
                {
                    if result.as_ref().is_ok_and(|f| f.ok) {
                        self.launching.remove(&app);
                    } else if next.step < FOLLOW_MS.len() {
                        self.follow = Some(next);
                    }
                }
                let key = letter as char;
                match result {
                    Ok(f) => {
                        self.activations.total += 1;
                        self.activations.ok += u32::from(f.ok);
                        self.activations.neutral_key += u32::from(f.neutral_key);
                        if f.ok && self.options.mouse_follows_focus {
                            out.push(Effect::MovePointer { win });
                        }
                        if f.ok {
                            self.map_focused(win, &mut out);
                        }
                        let status = if f.ok { "ok" } else { "FALLÓ" };
                        let trick = if f.neutral_key { ", con tecla neutra" } else { "" };
                        let since = now_us.saturating_sub(key_us) as f64 / 1000.0;
                        out.push(Effect::Log(format!(
                            "Caps + {key} → {app} «{}»: {status} en {:.1} ms{trick} \
                             (desde la tecla {since:.1} ms)",
                            shorten(&title, 50),
                            f.elapsed_us as f64 / 1000.0,
                        )));
                    }
                    Err(e) => out.push(Effect::Log(format!("Caps + {key}: {e}"))),
                }
            }
            Event::Inspected { req, result } => match self.pending.remove(&req) {
                Some(Pending::Arrange { dir, key_us, what, gesture, seen, continues }) => {
                    match result {
                        Ok(obs) => {
                            let g = (gesture, seen, continues);
                            self.arrange(dir, g, key_us, what, obs, now_us, &mut out);
                        }
                        Err(e) => out.push(Effect::Log(format!("{what}: {e}"))),
                    }
                }
                Some(Pending::Verify { win, job }) => {
                    self.verify(win, job, result, now_us, &mut out)
                }
                Some(Pending::Full { key_us, what }) => match result {
                    Ok(obs) => self.full(obs, what, key_us, now_us, &mut out),
                    Err(e) => out.push(Effect::Log(format!("{what}: {e}"))),
                },
                Some(Pending::Center { key_us, what }) => match result {
                    Ok(obs) => self.center(obs, what, key_us, now_us, &mut out),
                    Err(e) => out.push(Effect::Log(format!("{what}: {e}"))),
                },
                Some(Pending::Resize { key_us, what, grow }) => match result {
                    Ok(obs) => self.resize(obs, grow, what, key_us, now_us, &mut out),
                    Err(e) => out.push(Effect::Log(format!("{what}: {e}"))),
                },
                Some(Pending::Arrive { win }) => self.arrival_seen(win, result, now_us, &mut out),
                Some(Pending::Keep { letter }) => self.keep_seen(letter, result, &mut out),
                Some(Pending::Undo { key_us, entry }) => match result {
                    Ok(obs) => self.undo(entry, obs, key_us, now_us, &mut out),
                    // La ventana ya no existe: la entrada anterior, en la misma tecla.
                    Err(_) => self.undo_next(key_us, &mut out),
                },
                _ => {}
            },
            Event::Placed { req, result } => {
                if let Some(Pending::PlaceCmd { win, job }) = self.pending.remove(&req)
                    && let Err(e) = result
                    && self.jobs.get(&win).is_some_and(|j| j.id == job)
                {
                    let j = self.jobs.remove(&win).expect("recién visto");
                    out.push(Effect::Log(format!("{}: {e}", j.what)));
                }
            }
            Event::Flown { req, result } => self.flown(req, result, now_us, &mut out),
            Event::FlyEnabled(on) => self.fly = on,
            Event::Command(c) => self.command(c, now_us, &mut out),
            Event::Tick => self.tick(now_us, &mut out),
            Event::Boot { recover } => {
                self.start_sync(crate::desks::Goal::Boot { recover }, now_us, &mut out);
                self.screens_boot(now_us, &mut out);
            }
            Event::Screen(e) => self.on_screen(e, now_us, &mut out),
            Event::Surveyed { req, result } => {
                if let Some(Pending::Survey) = self.pending.get(&req) {
                    self.pending.remove(&req);
                    self.on_surveyed(req, result, now_us, &mut out);
                } else if let Some(&Pending::Regap { old_gap }) = self.pending.get(&req) {
                    self.pending.remove(&req);
                    self.on_regap(old_gap, result, now_us, &mut out);
                }
            }
            Event::ListHit { req, row } => {
                if let Some(Pending::ListHit) = self.pending.get(&req) {
                    self.pending.remove(&req);
                    self.list_clicked(row, now_us, &mut out);
                }
            }
            Event::Rescued { req, failed } => {
                if self.pending.remove(&req).is_some() {
                    self.on_rescued(failed, &mut out);
                }
            }
            Event::DesksSet { req, failed } => {
                if self.pending.remove(&req).is_some() {
                    self.desks_set(failed, &mut out);
                }
            }
            Event::Shown { req, result } => {
                if self.pending.remove(&req).is_some() {
                    self.sync_shown(result, now_us, &mut out);
                }
            }
            Event::Hidden { req, result } => {
                if self.pending.remove(&req).is_some() {
                    self.sync_hidden(result, now_us, &mut out);
                }
            }
            Event::Foreground(win) => self.on_foreground(win, now_us, &mut out),
            Event::Pointer { button, down, x, y } => {
                self.on_pointer(button, down, (x, y), now_us, &mut out)
            }
            Event::Grabbed { req, result } => {
                if let Some(Pending::Grab) = self.pending.get(&req) {
                    self.pending.remove(&req);
                    self.on_grabbed(req, result, &mut out);
                }
            }
            Event::DragDone { win, result } => self.on_drag_done(win, result, &mut out),
            Event::Options(o) => {
                let old = std::mem::replace(&mut self.options, o);
                if o.gap != old.gap {
                    self.regap(old.gap, &mut out);
                }
            }
            Event::Uncloaked(win) => self.on_uncloaked(win, now_us, &mut out),
            Event::Launched { req, result } => match self.pending.remove(&req) {
                Some(Pending::Launch { letter, app, path, retry }) => {
                    let key = letter as char;
                    match result {
                        Err(e) if e.starts_with(LOADING) => {
                            out.push(Effect::Log(format!(
                                "Caps + {key}: abriendo {app}… (espera)"
                            )));
                        }
                        Ok(()) if retry => {
                            out.push(Effect::Log(format!(
                                "Caps + {key} → abriendo {app} otra vez: no llegó a abrir ({path})"
                            )));
                            self.launching.insert(app.clone(), now_us + LAUNCH_GUARD_US);
                            self.start_follow(letter, now_us);
                        }
                        Ok(()) => {
                            out.push(Effect::Log(format!(
                                "Caps + {key} → abriendo {app} ({path})"
                            )));
                            self.start_follow(letter, now_us);
                        }
                        Err(e) => {
                            self.launching.remove(&app);
                            out.push(Effect::Log(format!(
                                "Caps + {key} → no se pudo abrir {app} ({path}): {e}"
                            )));
                        }
                    }
                }
                Some(Pending::Revive { letter, app }) => {
                    let key = letter as char;
                    match result {
                        Ok(()) => {
                            out.push(Effect::Log(format!(
                                "Caps + {key} → {app} no tenía ventana (¿en la bandeja?): la abro de nuevo"
                            )));
                            self.start_follow(letter, now_us);
                        }
                        Err(e) => {
                            self.launching.remove(&app);
                            out.push(Effect::Log(format!(
                                "Caps + {key}: no sé cómo abrir {app} ({e}); elegila de nuevo en Ajustes."
                            )));
                        }
                    }
                }
                _ => {}
            },
        }
        out
    }

    fn action(
        &mut self,
        action: HyperAction,
        held: u8,
        age_ms: u32,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        if !matches!(action, HyperAction::Letter { thumb: false, .. }) {
            self.drop_cycle(out);
        }
        match action {
            HyperAction::Letter { letter, thumb: false } => {
                let req = self.req();
                let key_us = now_us.saturating_sub(u64::from(age_ms) * 1000);
                self.pending.insert(req, Pending::Scan { letter, key_us });
                out.push(Effect::Scan { req });
            }
            HyperAction::Letter { letter, thumb: true } => {
                let what = format!("Caps + Alt + {}", letter as char);
                if let Some(dir) = Dir::from_letter(letter) {
                    // ¿Continúa un gesto? Una tecla nueva con todas las del
                    // gesto todavía apretadas (sin ninguna suelta en el medio).
                    let bit = dir.bit();
                    let (gesture, seen, continues) = match self.keys {
                        Some(k) if k.seen & bit == 0 && held == k.seen | bit => {
                            (k.id, k.seen | bit, true)
                        }
                        _ => {
                            self.next_gesture += 1;
                            (self.next_gesture, bit, false)
                        }
                    };
                    self.keys = Some(Keys { id: gesture, seen });
                    let req = self.req();
                    let key_us = now_us.saturating_sub(u64::from(age_ms) * 1000);
                    let pending = Pending::Arrange { dir, key_us, what, gesture, seen, continues };
                    self.pending.insert(req, pending);
                    out.push(Effect::Inspect { req, win: None });
                } else {
                    self.keys = None; // F, C, Z, G, V no forman acordes con las direcciones
                    let key_us = now_us.saturating_sub(u64::from(age_ms) * 1000);
                    match letter {
                        b'F' => {
                            let req = self.req();
                            self.pending.insert(req, Pending::Full { key_us, what });
                            out.push(Effect::Inspect { req, win: None });
                        }
                        b'C' => {
                            let req = self.req();
                            self.pending.insert(req, Pending::Center { key_us, what });
                            out.push(Effect::Inspect { req, win: None });
                        }
                        b'Z' => self.undo_next(key_us, out),
                        b'G' | b'V' => {
                            let req = self.req();
                            let grow = letter == b'G';
                            self.pending.insert(req, Pending::Resize { key_us, what, grow });
                            out.push(Effect::Inspect { req, win: None });
                        }
                        _ => out.push(Effect::Log(format!("{what}: todavía no hace nada."))),
                    }
                }
            }
            HyperAction::Digit { digit, thumb } => {
                self.keys = None;
                let key_us = now_us.saturating_sub(u64::from(age_ms) * 1000);
                if (1..=crate::desks::DESKS).contains(&digit) {
                    use crate::desks::Goal;
                    let goal = if thumb { Goal::Carry(digit) } else { Goal::Desk(digit) };
                    self.start_sync(goal, key_us, out);
                } else {
                    let alt = if thumb { "Alt + " } else { "" };
                    out.push(Effect::Log(format!("Caps + {alt}{digit}: hay 4 escritorios (1-4).")));
                }
            }
            HyperAction::Escape if self.esc_quits => out.push(Effect::Quit),
            HyperAction::Escape => {}
            HyperAction::Other { .. } => {}
        }
    }

    /// Capa de ventana: la ventana al frente ya observada (plan-core.md §3.1).
    #[allow(clippy::too_many_arguments)]
    fn arrange(
        &mut self,
        dir: Dir,
        (id, seen, continues): (u64, u8, bool),
        key_us: u64,
        what: String,
        obs: Obs,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        if obs.minimized {
            out.push(Effect::Log(format!("{what}: la ventana está minimizada.")));
            return;
        }
        let Some(m) = monitor_of(&obs.monitors, obs.frame) else {
            out.push(Effect::Log(format!("{what}: no hay monitores.")));
            return;
        };
        if !obs.maximized && obs.frame == obs.monitors[m].bounds {
            out.push(Effect::Log(format!("{what}: {} está en pantalla completa.", obs.app)));
            return;
        }
        let bit = dir.bit();
        // Continúa el gesto si así se decidió al llegar la tecla y la primera
        // tecla del gesto ya fijó el origen sobre esta misma ventana.
        let ongoing = self.gesture.filter(|g| continues && g.id == id && g.win == obs.key);
        if let Some(mut g) = ongoing {
            g.seen = seen;
            self.gesture = Some(g);
            if self.deferred.as_ref().is_some_and(|d| d.obs.key == obs.key) {
                self.deferred = None; // la primera tecla sola queda sin efecto
            }
            let what = format!("Caps + Alt + {}", chord_name(g.seen));
            self.chord(g, what, key_us, &obs, now_us, out);
            return;
        }

        // Empieza un gesto: primero lo que haya quedado pendiente.
        if let Some(d) = self.deferred.take() {
            self.run_deferred(d, now_us, out);
        }
        let (here, here_rect) = self.locate(&obs, m);
        let origin_max = obs.maximized && !self.jobs.contains_key(&obs.key);
        self.gesture = Some(Gesture {
            id,
            win: obs.key,
            origin_max,
            origin: here,
            origin_rect: here_rect,
            seen: bit,
            key_us,
        });
        let before = Entry {
            win: obs.key,
            frame: here_rect,
            maximized: origin_max,
            tag: id,
            desk: None,
            went: false,
        };
        match step(here.placement, dir) {
            // Desde una esquina, una tecla puede ser la primera de un acorde
            // a otra esquina (mismo tamaño: se anima): esperar la segunda.
            Move::Stay(p) if is_corner(here.placement) && !obs.maximized => {
                let to = DeferredTo::Here(Spot { monitor: here.monitor, placement: p });
                let due_us = now_us + CHORD_WAIT_US;
                self.deferred = Some(Deferred { due_us, before, obs, what, key_us, to });
            }
            Move::Stay(p) => {
                let to = Spot { monitor: here.monitor, placement: p };
                let what = format!("{what} → {} ({})", placement_name(p), obs.app);
                self.record = Some(before);
                self.go(&obs, to, self.slot(&to.monitor, p), what, key_us, now_us, out);
            }
            Move::Push { dir: push, enter } => {
                let n = index_of(&obs.monitors, here.monitor)
                    .and_then(|i| neighbor(&obs.monitors, i, push));
                let to = n.map(|n| Spot { monitor: obs.monitors[n], placement: enter });
                // Sin vecino y no exacta (a 1 px, por ejemplo tras un cambio
                // de escala): la tecla la deja exacta, salvo que ese marco sea
                // lo que la app logró cuando la pusimos.
                let fix = (obs.frame != self.slot(&here.monitor, here.placement)
                    && !self.remembered(&obs))
                .then_some(here);
                let to = DeferredTo::Push { to, dir: push, from: here.placement, fix };
                let due_us = now_us + CHORD_WAIT_US;
                self.deferred = Some(Deferred { due_us, before, obs, what, key_us, to });
            }
        }
    }

    /// Dos o más teclas juntas: la ubicación que nombran, calculada desde el
    /// origen del gesto. Si es donde ya estaba, empuja (primero de costado).
    fn chord(
        &mut self,
        g: Gesture,
        what: String,
        key_us: u64,
        obs: &Obs,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        let before = Entry {
            win: g.win,
            frame: g.origin_rect,
            maximized: g.origin_max,
            tag: g.id,
            desk: None,
            went: false,
        };
        // Más tarde que la espera: la primera tecla ya salió sola (se vio un
        // paso intermedio). Queda en el log para afinar la espera.
        let gap_us = key_us.saturating_sub(g.key_us);
        let what = if gap_us > CHORD_WAIT_US {
            format!("{what} (la segunda tecla llegó {} ms después)", gap_us / 1000)
        } else {
            what
        };
        let Some(target) = chord(g.seen) else {
            // Teclas opuestas: no nombran nada; vuelve a donde estaba (y el
            // gesto no deja nada para deshacer).
            if self.journal.back().is_some_and(|e| e.tag == g.id) {
                self.journal.pop_back();
            }
            let what = format!("{what} → se anulan ({})", obs.app);
            self.go(obs, g.origin, g.origin_rect, what, key_us, now_us, out);
            return;
        };
        self.record = Some(before);
        if target != g.origin.placement {
            let to = Spot { monitor: g.origin.monitor, placement: target };
            let what = format!("{what} → {} ({})", placement_name(target), obs.app);
            self.go(obs, to, self.slot(&to.monitor, target), what, key_us, now_us, out);
            return;
        }
        let from = index_of(&obs.monitors, g.origin.monitor);
        for push in chord_pushes(target) {
            if let Some(n) = from.and_then(|i| neighbor(&obs.monitors, i, push)) {
                let p = mirror(target, push);
                let to = Spot { monitor: obs.monitors[n], placement: p };
                let what = format!(
                    "{what} → monitor {}, {} ({})",
                    push.of_side(),
                    placement_name(p),
                    obs.app
                );
                self.go(obs, to, self.slot(&to.monitor, p), what, key_us, now_us, out);
                return;
            }
        }
        let what = format!("{what} → {} ({})", placement_name(target), obs.app);
        out.push(Effect::Log(format!("{what}: no hay otro monitor de ese lado.")));
        let exact = self.slot(&g.origin.monitor, target);
        if g.origin_rect != exact && !self.remembered(obs) {
            // A 1 px de su lugar: dejarla exacta, sin entrada en Z.
            self.record = None;
            self.exact = true;
            self.go(obs, g.origin, exact, format!("{what} (ajuste)"), key_us, now_us, out);
            return;
        }
        self.go(obs, g.origin, g.origin_rect, what, key_us, now_us, out);
    }

    /// El marco es el que la app logró cuando la pusimos (tamaño mínimo,
    /// redondeo propio): no hay nada que ajustar.
    fn remembered(&self, obs: &Obs) -> bool {
        self.placed.get(&obs.key).is_some_and(|(_, a)| edge_error(obs.frame, *a) <= TOLERANCE_PX)
    }

    fn run_deferred(&mut self, d: Deferred, now_us: u64, out: &mut Vec<Effect>) {
        let (to, dir, from, fix) = match d.to {
            DeferredTo::Here(to) => {
                let p = to.placement;
                let what = format!("{} → {} ({})", d.what, placement_name(p), d.obs.app);
                self.record = Some(d.before);
                self.go(&d.obs, to, self.slot(&to.monitor, p), what, d.key_us, now_us, out);
                return;
            }
            DeferredTo::Push { to, dir, from, fix } => (to, dir, from, fix),
        };
        match to {
            Some(to) => {
                let what = format!(
                    "{} → monitor {}, {} ({})",
                    d.what,
                    dir.of_side(),
                    placement_name(to.placement),
                    d.obs.app
                );
                let rect = self.slot(&to.monitor, to.placement);
                self.record = Some(d.before);
                self.go(&d.obs, to, rect, what, d.key_us, now_us, out);
            }
            None => {
                let what = format!("{} → {} ({})", d.what, placement_name(from), d.obs.app);
                out.push(Effect::Log(format!("{what}: no hay otro monitor {}.", dir.toward())));
                if let Some(spot) = fix {
                    self.record = None;
                    self.exact = true;
                    let rect = self.slot(&spot.monitor, spot.placement);
                    let what = format!("{what} (ajuste)");
                    self.go(&d.obs, spot, rect, what, d.key_us, now_us, out);
                }
            }
        }
    }

    /// Dónde está la ventana para decidir la próxima tecla: adonde va (si hay
    /// una colocación en curso), donde la dejamos (si no se movió), o lo que
    /// se ve en el monitor (mitad, esquina, o libre).
    fn locate(&self, obs: &Obs, m: usize) -> (Spot, Rect) {
        let monitor = obs.monitors[m];
        if let Some(j) = self.jobs.get(&obs.key) {
            return (j.spot, j.target);
        }
        if let Some((spot, achieved)) = self.placed.get(&obs.key)
            && edge_error(obs.frame, *achieved) <= TOLERANCE_PX
            && index_of(&obs.monitors, spot.monitor).is_some()
            && !obs.maximized
        {
            return (*spot, *achieved);
        }
        let placement = if obs.maximized {
            Placement::Full
        } else {
            slot_at(&monitor, obs.frame, self.options.gap).unwrap_or(Placement::Full)
        };
        (Spot { monitor, placement }, obs.frame)
    }

    /// Lleva la ventana a `target` (el marco visible), salvo que ya esté ahí o
    /// ya vaya hacia ahí.
    #[allow(clippy::too_many_arguments)]
    fn go(
        &mut self,
        obs: &Obs,
        spot: Spot,
        target: Rect,
        what: String,
        key_us: u64,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        let record = self.record.take();
        let tolerance = if std::mem::take(&mut self.exact) { 0 } else { TOLERANCE_PX };
        // El ancla de G/V vale solo si el último movimiento de la ventana fue
        // G o V: con cualquier otro (A/D/W/S, esquina, C, Z) se olvida. Antes
        // se deducía por la posición y una mitad recién puesta podía pasar
        // por "la misma de antes" (pruebas del usuario, 16/11/2026).
        if !std::mem::take(&mut self.sizing_move) {
            self.sizing.remove(&obs.key);
        }
        // Un rectángulo concreto (deshacer, centrar) que coincide con una mitad
        // o esquina cuenta como esa ubicación para la tecla siguiente.
        let spot = if spot.placement == Placement::Full {
            let placement =
                slot_at(&spot.monitor, target, self.options.gap).unwrap_or(Placement::Full);
            Spot { placement, ..spot }
        } else {
            spot
        };
        if self.jobs.get(&obs.key).is_some_and(|j| j.target == target) {
            return; // ya va hacia ahí
        }
        if !obs.maximized
            && edge_error(obs.frame, target) <= tolerance
            && !self.jobs.contains_key(&obs.key)
        {
            self.placed.insert(obs.key, (spot, obs.frame));
            return; // ya está ahí
        }
        if let Some(entry) = record {
            self.remember(entry);
        }
        let fly = self.may_fly(obs, spot, target);
        if !fly {
            self.ground(obs.key, out);
        }
        if obs.maximized {
            out.push(Effect::Normalize { win: obs.key });
        }
        let rect = outer_for(obs.frame, obs.outer, target, obs.dpi, spot.monitor.dpi);
        self.next_job += 1;
        let job = Job {
            id: self.next_job,
            what,
            target,
            key_us,
            pass: 1,
            placed_us: now_us,
            check: 0,
            inspecting: false,
            last_frame: None,
            from_frame: obs.frame,
            want_dpi: spot.monitor.dpi,
            spot,
            anchored: false,
            flying: fly,
            flight_ms: None,
        };
        // La última tecla gana: la colocación anterior de esta ventana se olvida.
        self.jobs.insert(obs.key, job);
        if fly {
            let req = self.req();
            let (win, job) = (obs.key, self.next_job);
            self.pending.insert(req, Pending::FlyCmd { win, job, rect });
            let ms = fly_ms(obs.frame, target);
            out.push(Effect::Fly { req, win, rect, from: obs.frame, to: target, ms });
        } else {
            self.send_place(obs.key, rect, out);
        }
    }

    /// Se anima si la ventana está normal, al frente, en la misma escala y lo
    /// bastante lejos. Cambiar de tamaño también vuela (el carril recorta la
    /// miniatura, mini-spike 6b, architecture.md §13.8b), salvo a un tamaño
    /// que la ventana ya mostró que no toma (mínimo o fijo): terminaría en
    /// otro lado y daría un salto al final.
    fn may_fly(&self, obs: &Obs, spot: Spot, target: Rect) -> bool {
        let resize = target.width() != obs.frame.width() || target.height() != obs.frame.height();
        self.fly
            && self.options.animate
            && !self.screens.frozen()
            && obs.no_fly.is_none()
            && !obs.maximized
            && !obs.minimized
            && !(resize && self.limits.get(&obs.key).is_some_and(|l| !l.allows(target)))
            && obs.dpi == spot.monitor.dpi
            && travel(obs.frame, target) >= FLY_MIN_PX
    }

    /// Si la ventana está en vuelo, bajarla ya (antes de otro pedido).
    fn ground(&mut self, win: WinKey, out: &mut Vec<Effect>) {
        if let Some(j) = self.jobs.get_mut(&win).filter(|j| j.flying) {
            j.flying = false;
            out.push(Effect::Land { win });
        }
    }

    /// Terminó un vuelo: verificar ya dónde quedó la real. Si no se pudo
    /// volar, el mismo pedido sin animación.
    fn flown(
        &mut self,
        req: Req,
        result: Result<FlyReport, FlyFail>,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        let Some(Pending::FlyCmd { win, job, rect }) = self.pending.remove(&req) else {
            return;
        };
        self.drag_after_landing(out);
        if let Err(f) = &result
            && f.fatal
            && self.fly
        {
            self.fly = false;
            out.push(Effect::Log(format!("Animación apagada por esta sesión: {}", f.why)));
        }
        let Some(j) = self.jobs.get_mut(&win).filter(|j| j.id == job && j.flying) else {
            return; // otra tecla la redirigió o la bajó
        };
        j.flying = false;
        j.placed_us = now_us;
        j.check = 0;
        match result {
            Ok(report) => {
                j.flight_ms = Some(report.flight_ms);
                j.inspecting = true;
                let req = self.req();
                self.pending.insert(req, Pending::Verify { win, job });
                out.push(Effect::Inspect { req, win: Some(win) });
            }
            Err(_) => self.send_place(win, rect, out),
        }
    }

    /// Anota una entrada en el historial (una por gesto).
    fn remember(&mut self, entry: Entry) {
        if self.journal.back().is_some_and(|e| e.tag == entry.tag && e.win == entry.win) {
            return;
        }
        if self.journal.len() == JOURNAL_MAX {
            self.journal.pop_front();
        }
        self.journal.push_back(entry);
    }

    fn fresh_tag(&mut self) -> u64 {
        self.next_gesture += 1;
        self.next_gesture
    }

    /// F: maximizar (como Windows); si ya está maximizada, volver al tamaño
    /// anterior (Windows lo recuerda).
    fn full(&mut self, obs: Obs, what: String, key_us: u64, now_us: u64, out: &mut Vec<Effect>) {
        if obs.minimized {
            out.push(Effect::Log(format!("{what}: la ventana está minimizada.")));
            return;
        }
        if is_fullscreen(&obs) {
            out.push(Effect::Log(format!("{what}: {} está en pantalla completa.", obs.app)));
            return;
        }
        let tag = self.fresh_tag();
        self.remember(Entry {
            win: obs.key,
            frame: obs.frame,
            maximized: world_rules::looks_maximized(&obs),
            tag,
            desk: None,
            went: false,
        });
        self.ground(obs.key, out);
        self.sizing.remove(&obs.key);
        self.jobs.remove(&obs.key); // una colocación en curso ya no vale
        self.placed.remove(&obs.key);
        let since = now_us.saturating_sub(key_us) as f64 / 1000.0;
        if world_rules::looks_maximized(&obs) {
            out.push(Effect::Normalize { win: obs.key });
            out.push(Effect::Log(format!(
                "{what} → tamaño anterior ({}), {since:.1} ms desde la tecla",
                obs.app
            )));
        } else {
            if obs.maximized {
                // Dice estar maximizada sin estarlo (Steam): sacarle la marca.
                out.push(Effect::Normalize { win: obs.key });
            }
            out.push(Effect::Maximize { win: obs.key });
            out.push(Effect::Log(format!(
                "{what} → maximizada ({}), {since:.1} ms desde la tecla",
                obs.app
            )));
        }
    }

    /// G/V: agrandar o achicar un 10 % del área de trabajo, pegada al borde
    /// exterior de donde está (plan-core.md, hito 1f). Una mitad izquierda o
    /// derecha cambia el ancho; una de arriba o de abajo, el alto; una
    /// esquina, los dos, contra la esquina; una ventana libre, los dos,
    /// alrededor de su centro. Repetir sigue con la misma ancla aunque ya no
    /// sea una mitad exacta. Maximizada cuenta como libre del tamaño del área.
    fn resize(
        &mut self,
        obs: Obs,
        grow: bool,
        what: String,
        key_us: u64,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        if obs.minimized {
            out.push(Effect::Log(format!("{what}: la ventana está minimizada.")));
            return;
        }
        let Some(m) = monitor_of(&obs.monitors, obs.frame) else { return };
        let monitor = obs.monitors[m];
        // v1.1 (d): sobre el tablero (el área menos el espacio).
        let work = self.board(&monitor);
        if !obs.maximized && obs.frame == monitor.bounds {
            out.push(Effect::Log(format!("{what}: {} está en pantalla completa.", obs.app)));
            return;
        }
        let (_, here_rect) = self.locate(&obs, m);
        let here_rect = if obs.maximized { work } else { here_rect };
        // Seguir con la misma ancla si la ventana está donde la dejó la
        // tecla anterior; si no, el lugar donde está ahora.
        let previous = self
            .sizing
            .get(&obs.key)
            // Donde la dejó la tecla anterior: el lugar pedido o, si la app
            // no aceptó ese tamaño, pegada al mismo borde con el que logró.
            .filter(|s| {
                let (w, h) = (here_rect.width(), here_rect.height());
                !obs.maximized
                    && (edge_error(here_rect, s.target) <= TOLERANCE_PX
                        || edge_error(here_rect, anchor(s.target, work, w, h)) <= TOLERANCE_PX)
            })
            .copied();
        let (anchor_rect, x, y) = match previous {
            Some(s) => (s.anchor, s.x, s.y),
            None => {
                // Según los bordes del área que toca (no solo mitades y
                // esquinas exactas: una franja del 40 % que volvió con Z
                // también cambia solo el ancho, prueba del usuario).
                // Toca un borde: entre el área y el tablero (±1 px).
                let full = monitor.work;
                let near = |a: i32, edge: i32, inner: i32| {
                    a >= edge.min(inner) - TOLERANCE_PX && a <= edge.max(inner) + TOLERANCE_PX
                };
                let (l, r) = (
                    near(here_rect.left, full.left, work.left),
                    near(here_rect.right, full.right, work.right),
                );
                let (t, b) = (
                    near(here_rect.top, full.top, work.top),
                    near(here_rect.bottom, full.bottom, work.bottom),
                );
                let (side_x, side_y) = (l != r, t != b);
                let (full_x, full_y) = (l && r, t && b);
                let (x, y) = if full_y && side_x {
                    (true, false)
                } else if full_x && side_y {
                    (false, true)
                } else {
                    (true, true)
                };
                // `anchor` pega al borde que toca este rectángulo (o centra).
                (here_rect, x, y)
            }
        };
        let size = |current: i32, total: i32, on: bool| {
            if !on {
                return current;
            }
            let step = total * SIZE_STEP_PCT / 100;
            let next = if grow { current + step } else { current - step };
            next.clamp(total * SIZE_MIN_PCT / 100, total)
        };
        let (w, h) =
            (size(here_rect.width(), work.width(), x), size(here_rect.height(), work.height(), y));
        // Sin pedir lo que la app ya mostró que no acepta (tamaño mínimo o
        // fijo): en la prueba del usuario, cada V de más tardaba ~150 ms en
        // terminar donde mismo.
        let lim = self.limits.get(&obs.key).copied().unwrap_or_default();
        let clamp = |v: i32, min: Option<i32>, max: Option<i32>| {
            let v = min.map_or(v, |m| v.max(m));
            max.map_or(v, |m| v.min(m))
        };
        let (w, h) = (clamp(w, lim.min_w, lim.max_w), clamp(h, lim.min_h, lim.max_h));
        let near = |a: i32, b: i32| (a - b).abs() <= TOLERANCE_PX;
        if near(w, here_rect.width()) && near(h, here_rect.height()) {
            let at_app_limit = if grow {
                lim.max_w.is_some() || lim.max_h.is_some()
            } else {
                lim.min_w.is_some() || lim.min_h.is_some()
            };
            let limit = match (grow, at_app_limit) {
                (true, false) => "ya está en el tamaño máximo",
                (false, false) => "ya está en el tamaño mínimo",
                (true, true) => "la app no se agranda más",
                (false, true) => "la app no se achica más",
            };
            let (cw, ch) = (here_rect.width(), here_rect.height());
            out.push(Effect::Log(format!("{what}: {limit} ({}, {cw}×{ch}).", obs.app)));
            return;
        }
        let target = fit_inside(anchor(anchor_rect, work, w, h), work);
        self.sizing.insert(obs.key, Sizing { anchor: anchor_rect, x, y, target });
        let pct = |a: i32, b: i32| (f64::from(a) * 100.0 / f64::from(b)).round() as i32;
        let label = match (x, y) {
            (true, false) => format!("{} % de ancho", pct(w, work.width())),
            (false, true) => format!("{} % de alto", pct(h, work.height())),
            _ => format!("{} % × {} %", pct(w, work.width()), pct(h, work.height())),
        };
        let verb = if grow { "más grande" } else { "más chica" };
        let tag = self.fresh_tag();
        self.record = Some(Entry {
            win: obs.key,
            frame: obs.frame,
            maximized: obs.maximized,
            tag,
            desk: None,
            went: false,
        });
        let spot = Spot { monitor, placement: Placement::Full };
        let what = format!("{what} → {verb}, {label} ({})", obs.app);
        self.sizing_move = true;
        self.go(&obs, spot, target, what, key_us, now_us, out);
        self.sizing_move = false;
    }

    /// C: centrar sin cambiar el tamaño; repetir alterna 60 % → 80 % →
    /// tamaño original, siempre centrada.
    fn center(&mut self, obs: Obs, what: String, key_us: u64, now_us: u64, out: &mut Vec<Effect>) {
        if obs.minimized {
            out.push(Effect::Log(format!("{what}: la ventana está minimizada.")));
            return;
        }
        // Un video o juego a pantalla completa ignora los pedidos de mover: se
        // daba por colgada (prueba del usuario con Brave).
        if is_fullscreen(&obs) {
            out.push(Effect::Log(format!("{what}: {} está en pantalla completa.", obs.app)));
            return;
        }
        let Some(m) = monitor_of(&obs.monitors, obs.frame) else { return };
        let work = obs.monitors[m].work;
        // Seguir el ciclo si la ventana está donde la dejó la C anterior.
        let current = self
            .centers
            .get(&obs.key)
            .filter(|c| !obs.maximized && edge_error(obs.frame, c.target) <= TOLERANCE_PX)
            .copied();
        let next = |step: CenterStep| match step {
            CenterStep::Original => CenterStep::P60,
            CenterStep::P60 => CenterStep::P80,
            CenterStep::P80 => CenterStep::Original,
        };
        let (original, mut step) = match current {
            Some(c) => (c.original, next(c.step)),
            // Maximizada: no hay un tamaño propio que recordar; el "original"
            // es el 60 % (y abajo se saltea por ser igual).
            None if obs.maximized => {
                ((work.width() * 60 / 100, work.height() * 60 / 100), CenterStep::P60)
            }
            None => ((obs.frame.width(), obs.frame.height()), CenterStep::Original),
        };
        let rect_of = |step: CenterStep| match step {
            CenterStep::Original => (centered(work, original.0, original.1), "centrada"),
            CenterStep::P60 => (centered_percent(work, 60), "centrada al 60 %"),
            CenterStep::P80 => (centered_percent(work, 80), "centrada al 80 %"),
        };
        // Cada C tiene que mover algo: se saltean los pasos que dan el mismo
        // rectángulo donde ya está (ya centrada con su tamaño, o un "original"
        // igual al 60 %).
        // Si el "original" es el 60 % (empezó maximizada), ese paso sobra.
        let original_is_60 =
            edge_error(rect_of(CenterStep::Original).0, rect_of(CenterStep::P60).0) <= TOLERANCE_PX;
        for _ in 0..3 {
            let same_place =
                !obs.maximized && edge_error(obs.frame, rect_of(step).0) <= TOLERANCE_PX;
            if same_place || (step == CenterStep::Original && original_is_60) {
                step = next(step);
            }
        }
        let (target, label) = rect_of(step);
        self.centers.insert(obs.key, Centering { original, step, target });
        let spot = Spot { monitor: obs.monitors[m], placement: Placement::Full };
        let tag = self.fresh_tag();
        self.record = Some(Entry {
            win: obs.key,
            frame: obs.frame,
            maximized: obs.maximized,
            tag,
            desk: None,
            went: false,
        });
        let what = format!("{what} → {label} ({})", obs.app);
        self.go(&obs, spot, target, what, key_us, now_us, out);
    }

    /// Z: toma la última entrada del historial y pide la ventana.
    /// Una orden de la bandeja.
    fn command(&mut self, c: Command, now_us: u64, out: &mut Vec<Effect>) {
        match c {
            Command::Quit => out.push(Effect::Quit),
            Command::SetPaused(on) if on == self.paused => {}
            Command::SetPaused(on) => {
                self.paused = on;
                // Lo que estaba a medio hacer se olvida: el mapa, la capa y
                // un empuje demorado.
                let mut m = Vec::new();
                self.map.reset(&mut m);
                self.map_effects(m, out);
                self.layer = Layer::Off;
                self.keys = None;
                self.deferred = None;
                out.push(Effect::SetPaused(on));
                out.push(Effect::Log(
                    if on { "Atajos en pausa." } else { "Atajos reanudados." }.to_owned(),
                ));
            }
            // Igual que Z: la entrada del diario ya dice qué ventana era (no
            // hace falta que esté al frente). También en pausa: es un clic.
            Command::Undo => self.undo_next(now_us, out),
            Command::ShowAll => {
                self.start_sync(crate::desks::Goal::ShowAll, now_us, out);
                // Hito 4d.6: el botón de pánico también trae lo que quedó
                // fuera de pantalla (sin excepciones por "tercas").
                self.screens_show_all(now_us, out);
            }
            Command::Reconcile => self.start_sync(crate::desks::Goal::Verify, now_us, out),
            Command::Desk(n) => self.start_sync(crate::desks::Goal::Desk(n), now_us, out),
            Command::OpenLikeThis { win, letter } => self.keep_like_this(win, letter, out),
        }
    }

    fn undo_next(&mut self, key_us: u64, out: &mut Vec<Effect>) {
        // Un arrastre que todavía termina va a anotar su entrada: Z la espera
        // (si no, deshacía la anterior; visto en la CI).
        if self.dragging.is_some() && self.pointer.is_none() {
            self.undo_after_drag = Some(key_us);
            return;
        }
        match self.journal.pop_back() {
            Some(Entry { win, desk: Some(to), went, .. }) => {
                let goal = if went {
                    crate::desks::Goal::Bring { win, to, undo: true }
                } else {
                    crate::desks::Goal::UndoCarry { win, to }
                };
                self.start_sync(goal, key_us, out);
            }
            Some(entry) => {
                let req = self.req();
                self.pending.insert(req, Pending::Undo { key_us, entry });
                out.push(Effect::Inspect { req, win: Some(entry.win) });
            }
            None => out.push(Effect::Log("Caps + Alt + Z: no hay nada para deshacer.".into())),
        }
    }

    /// Z: deja la ventana como estaba antes de la entrada.
    fn undo(&mut self, entry: Entry, obs: Obs, key_us: u64, now_us: u64, out: &mut Vec<Effect>) {
        let what = format!("Caps + Alt + Z → deshacer ({})", obs.app);
        if is_fullscreen(&obs) {
            // La entrada queda para cuando salga de pantalla completa.
            self.journal.push_back(entry);
            out.push(Effect::Log(format!("{what}: {} está en pantalla completa.", obs.app)));
            return;
        }
        self.centers.remove(&obs.key);
        if entry.maximized {
            self.ground(obs.key, out);
            self.sizing.remove(&obs.key);
            self.jobs.remove(&obs.key);
            self.placed.remove(&obs.key);
            // Maximizada en otro monitor (la arrastraron o la empujaron
            // después): primero llevarla a aquel, y maximizarla ahí. Windows
            // maximiza en el monitor donde está la ventana.
            let here = monitor_of(&obs.monitors, obs.frame);
            let there = monitor_of(&obs.monitors, entry.frame);
            let moved = match (here, there) {
                (Some(h), Some(t)) if h != t => {
                    self.maximize_on(&obs, t, out);
                    true
                }
                _ => false,
            };
            if !moved && !obs.maximized {
                out.push(Effect::Maximize { win: obs.key });
            }
            let since = now_us.saturating_sub(key_us) as f64 / 1000.0;
            out.push(Effect::Log(format!("{what}: maximizada, {since:.1} ms desde la tecla")));
            return;
        }
        // El monitor de la entrada puede ya no estar (hito 4d): el destino se
        // trae a la pantalla más cercana; si vuelve, Z es exacta otra vez.
        let target = crate::screens::on_screen(&obs.monitors, entry.frame);
        if target != entry.frame {
            out.push(Effect::Log(format!(
                "{what}: su lugar quedó fuera de las pantallas actuales; la dejo en la más cercana."
            )));
        }
        let Some(m) = monitor_of(&obs.monitors, target) else { return };
        let spot = Spot { monitor: obs.monitors[m], placement: Placement::Full };
        self.go(&obs, spot, target, what, key_us, now_us, out);
    }

    /// v1.1 (d): una mitad o esquina de `m` con el espacio vigente.
    fn slot(&self, m: &MonitorObs, p: Placement) -> Rect {
        slot_rect(m, p, self.options.gap)
    }

    /// v1.1 (d): el área de `m` menos el espacio vigente.
    fn board(&self, m: &MonitorObs) -> Rect {
        crate::place::board(m, self.options.gap)
    }

    /// Maximizarla en el monitor `t` (no es el suyo): sacarle la marca,
    /// llevarla centrada a ese monitor y maximizarla ahí (Windows maximiza
    /// en el monitor donde está la ventana).
    fn maximize_on(&mut self, obs: &Obs, t: usize, out: &mut Vec<Effect>) {
        if obs.maximized {
            out.push(Effect::Normalize { win: obs.key });
        }
        let work = obs.monitors[t].work;
        let (w, h) = (obs.frame.width().min(work.width()), obs.frame.height().min(work.height()));
        let left = work.left + (work.width() - w) / 2;
        let top = work.top + (work.height() - h) / 2;
        let target = Rect::new(left, top, left + w, top + h);
        let rect = outer_for(obs.frame, obs.outer, target, obs.dpi, obs.dpi);
        // Sin chequeos: enseguida se maximiza.
        let req = self.req();
        out.push(Effect::Place { req, win: obs.key, rect });
        out.push(Effect::Maximize { win: obs.key });
    }

    fn send_place(&mut self, win: WinKey, rect: Rect, out: &mut Vec<Effect>) {
        let job = self.jobs[&win].id;
        let req = self.req();
        self.pending.insert(req, Pending::PlaceCmd { win, job });
        out.push(Effect::Place { req, win, rect });
    }

    /// Lanza los chequeos vencidos.
    fn tick(&mut self, now_us: u64, out: &mut Vec<Effect>) {
        self.pointer_tick(now_us, out);
        self.follow_tick(now_us, out);
        self.arrival_tick(now_us, out);
        self.unseen_tick(now_us, out);
        self.list_tick(now_us, out);
        self.screens_tick(now_us, out);
        if self.verify_due.is_some_and(|t| t <= now_us) {
            self.verify_due = None;
            self.start_sync(crate::desks::Goal::Verify, now_us, out);
        }
        let mut m = Vec::new();
        self.map.tick(self.layer, now_us, &mut m);
        self.map_effects(m, out);
        if self.deferred.as_ref().is_some_and(|d| d.due_us <= now_us) {
            let d = self.deferred.take().expect("recién visto");
            self.run_deferred(d, now_us, out);
        }
        let due: Vec<(WinKey, u64)> = self
            .jobs
            .iter()
            .filter(|(_, j)| !j.inspecting && !j.flying && next_check_us(j) <= now_us)
            .map(|(w, j)| (*w, j.id))
            .collect();
        for (win, job) in due {
            if let Some(j) = self.jobs.get_mut(&win) {
                j.inspecting = true;
            }
            let req = self.req();
            self.pending.insert(req, Pending::Verify { win, job });
            out.push(Effect::Inspect { req, win: Some(win) });
        }
    }

    fn verify(
        &mut self,
        win: WinKey,
        job: u64,
        result: Result<Obs, String>,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        let Some(j) = self.jobs.get_mut(&win).filter(|j| j.id == job) else {
            return; // una colocación más nueva la reemplazó
        };
        j.inspecting = false;
        let obs = match result {
            Ok(obs) => obs,
            Err(e) => {
                let what = j.what.clone();
                self.jobs.remove(&win);
                out.push(Effect::Log(format!("{what}: {e}")));
                return;
            }
        };
        let error = edge_error(obs.frame, j.target);
        let stable = j.last_frame == Some(obs.frame);
        j.last_frame = Some(obs.frame);
        j.check += 1;
        let exhausted = j.check >= CHECKS_MS.len();
        let since = |j: &Job| now_us.saturating_sub(j.key_us) as f64 / 1000.0;
        if error <= TOLERANCE_PX {
            let passes =
                if j.pass == 1 { "1 pasada".to_owned() } else { format!("{} pasadas", j.pass) };
            let flight = j.flight_ms.map(|ms| format!(" (vuelo {ms} ms)")).unwrap_or_default();
            let line = if j.anchored {
                let (w, h) = (obs.frame.width(), obs.frame.height());
                format!(
                    "{}: la app no se achica ({w}×{h}); quedó pegada al borde en {passes}, {:.1} ms{flight}",
                    j.what,
                    since(j)
                )
            } else {
                format!(
                    "{}: {error} px en {passes}, {:.1} ms desde la tecla{flight}",
                    j.what,
                    since(j)
                )
            };
            let spot = j.spot;
            self.jobs.remove(&win);
            self.placed.insert(win, (spot, obs.frame));
            out.push(Effect::Log(line));
            return;
        }
        let moved = obs.frame != j.from_frame;
        // Al cruzar de escala, la app se reescala sola al recibir el DPI nuevo:
        // hasta que lo informe, el marco todavía no es el final.
        let dpi_pending = obs.dpi != j.want_dpi;
        // En la primera pasada, "quieta en otro lugar" exige que se haya movido
        // (si no, la app todavía no aplicó el pedido). Desde la segunda, quieta
        // alcanza: una app con tamaño mínimo o fijo no cambia al repetirle el
        // pedido, y esperar a que "se mueva" costaba hasta el último chequeo.
        let settled = stable && (moved || j.pass >= 2) && !dpi_pending;
        if !(settled || exhausted) {
            return; // la app todavía no aplicó el pedido o se está acomodando
        }
        if j.pass == 1 && !moved {
            // Ni se movió, pero ya está donde quedaría pegada al borde con el
            // tamaño que tiene (tamaño fijo o mínimo que ya estaba en ese
            // lugar): no hay nada que hacer, y la app no está colgada.
            let (w, h) = (obs.frame.width(), obs.frame.height());
            if edge_error(
                obs.frame,
                anchor(j.target, crate::place::board(&j.spot.monitor, self.options.gap), w, h),
            ) <= TOLERANCE_PX
            {
                self.limits.entry(win).or_default().learn(obs.frame, j.target);
                let line = format!(
                    "{}: la app no se achica ({w}×{h}); ya estaba pegada al borde, {:.1} ms",
                    j.what,
                    since(j)
                );
                let spot = j.spot;
                self.jobs.remove(&win);
                self.placed.insert(win, (spot, obs.frame));
                out.push(Effect::Log(line));
                return;
            }
            // A los 600 ms ni se movió: la app no responde (colgada). El
            // pedido ya está en su cola y se aplica solo cuando despierte; no
            // se reintenta ni se ancla (plan-core.md §3.3).
            let line = format!(
                "{}: la app no responde; va a llegar sola cuando despierte ({:.1} ms)",
                j.what,
                since(j)
            );
            let (spot, target) = (j.spot, j.target);
            self.jobs.remove(&win);
            self.placed.insert(win, (spot, target));
            out.push(Effect::Log(line));
            return;
        }
        if j.pass == 1 {
            // Segunda pasada con los márgenes que tiene ahora (cambian al
            // cruzar de DPI o al salir de maximizada).
            j.pass = 2;
            j.placed_us = now_us;
            j.check = 0;
            j.last_frame = None;
            j.from_frame = obs.frame;
            let dpi = obs.dpi;
            let rect = outer_for(obs.frame, obs.outer, j.target, dpi, dpi);
            self.send_place(win, rect, out);
            return;
        }
        let (w, h) = (obs.frame.width(), obs.frame.height());
        // La app no tomó el tamaño (mínimo o fijo): una pasada más, con el
        // tamaño que logró, pegada al borde exterior del lugar pedido.
        let resized = (w - j.target.width()).abs() > TOLERANCE_PX
            || (h - j.target.height()).abs() > TOLERANCE_PX;
        if resized && stable {
            self.limits.entry(win).or_default().learn(obs.frame, j.target);
        }
        if resized && !j.anchored && stable {
            let anchored =
                anchor(j.target, crate::place::board(&j.spot.monitor, self.options.gap), w, h);
            j.anchored = true;
            if edge_error(obs.frame, anchored) > TOLERANCE_PX {
                j.target = anchored;
                j.pass += 1;
                j.placed_us = now_us;
                j.check = 0;
                j.last_frame = None;
                j.from_frame = obs.frame;
                let rect = outer_for(obs.frame, obs.outer, anchored, obs.dpi, obs.dpi);
                self.send_place(win, rect, out);
                return;
            }
            j.target = anchored;
        }
        let error = edge_error(obs.frame, j.target);
        let line = if j.anchored && error <= TOLERANCE_PX {
            format!(
                "{}: la app no se achica ({w}×{h}); quedó pegada al borde, {:.1} ms",
                j.what,
                since(j)
            )
        } else {
            format!(
                "{}: quedó a {error} px ({w}×{h}; tamaño mínimo o app que no responde), {:.1} ms",
                j.what,
                since(j)
            )
        };
        let spot = j.spot;
        self.jobs.remove(&win);
        self.placed.insert(win, (spot, obs.frame));
        out.push(Effect::Log(line));
    }

    fn go_to_app(
        &mut self,
        letter: u8,
        key_us: u64,
        windows: &[WinInfo],
        foreground: Option<WinKey>,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        let list: Vec<(WinKey, crate::apps::AppId)> =
            windows.iter().map(|w| (w.key, w.app_id())).collect();
        let key = letter as char;
        match resolve(letter, &self.bindings, &list) {
            Target::Focus { windows: order, app } => {
                self.launching.remove(&app);
                // Segundo toque (o más) con 3 o más ventanas: modo lista.
                if self.map.enabled()
                    && let Some(c) = self.cycle.as_ref().filter(|c| c.letter == letter)
                    && c.order.iter().filter(|w| order.contains(w)).count() >= 3
                {
                    let snapshot = (windows.to_vec(), foreground);
                    self.list_move(1, Some(snapshot), now_us, out);
                    return;
                }
                if self.cycle.as_ref().is_some_and(|c| c.letter != letter) {
                    self.drop_cycle(out);
                }
                let Some(window) = self.pick(letter, order, foreground) else {
                    out.push(Effect::Log(format!("Caps + {key}: {app} es la única ventana.")));
                    return;
                };
                self.focus_window(letter, key_us, window, &app, windows, foreground, now_us, out);
            }
            Target::Launch { app, path } => {
                // Durante la espera, se pide abrir "solo si ya no corre": si
                // el intento anterior falló (la app no llegó a abrir), se
                // abre de nuevo; si sigue cargando, solo se avisa.
                let retry = self.launching.get(&app).is_some_and(|&until| until > now_us);
                if !retry {
                    self.launching.insert(app.clone(), now_us + LAUNCH_GUARD_US);
                }
                let req = self.req();
                let pending =
                    Pending::Launch { letter, app: app.clone(), path: path.clone(), retry };
                self.pending.insert(req, pending);
                self.follow_fg = foreground;
                let names = self
                    .bindings
                    .iter()
                    .find(|b| b.letter == letter.to_ascii_uppercase())
                    .map(|b| b.names().map(str::to_owned).collect())
                    .unwrap_or_default();
                out.push(Effect::Launch { req, path, names, only_if_closed: retry });
            }
            Target::Revive { app, names } => {
                if self.guarded(&app, key, now_us, out) {
                    return;
                }
                self.launching.insert(app.clone(), now_us + LAUNCH_GUARD_US);
                let req = self.req();
                self.pending.insert(req, Pending::Revive { letter, app });
                self.follow_fg = foreground;
                out.push(Effect::Revive { req, names });
            }
            Target::Nothing => {
                out.push(Effect::Log(format!(
                    "Caps + {key}: no hay ninguna app abierta con la {key}."
                )));
            }
        }
    }

    /// "El Alt+Tab de la letra" (plan-apps.md §5): con `Caps` todavía abajo
    /// desde la tecla anterior de esta letra, la siguiente de la lista
    /// congelada; si no, la más reciente, o la anterior si ya estás en una.
    /// `None`: es la única y ya está al frente.
    fn pick(&mut self, letter: u8, order: Vec<WinKey>, fg: Option<WinKey>) -> Option<WinKey> {
        if let Some(c) = self.cycle.as_mut().filter(|c| c.letter == letter) {
            // Saltear las que se cerraron en el medio.
            let alive: Vec<WinKey> =
                c.order.iter().copied().filter(|w| order.contains(w)).collect();
            if alive.len() > 1 {
                let cur = c.order.get(c.pos).copied();
                let i = cur.and_then(|w| alive.iter().position(|&x| x == w)).unwrap_or(0);
                let next = alive[(i + 1) % alive.len()];
                c.pos = c.order.iter().position(|&w| w == next).unwrap_or(0);
                return Some(next);
            }
        }
        let target = match fg.filter(|f| order.contains(f)) {
            Some(f) => order.iter().copied().find(|&w| w != f)?,
            None => *order.first()?,
        };
        let pos = order.iter().position(|&w| w == target).unwrap_or(0);
        // Si Caps ya se soltó antes de que llegara el `Scan`, no queda un
        // ciclo viejo para la próxima pulsación.
        if self.layer != Layer::Off {
            self.cycle = Some(Cycle {
                letter,
                order,
                pos,
                list: false,
                list_due: None,
                list_shown: false,
                list_since: 0,
                front: Some(target),
                snapshot: Vec::new(),
                fg: None,
            });
        }
        Some(target)
    }

    /// Modo lista: mover la selección `delta` (y la vuelta) entre las
    /// ventanas que siguen abiertas; `snapshot`: un `Scan` nuevo (títulos).
    fn list_move(
        &mut self,
        delta: i64,
        snapshot: Option<(Vec<WinInfo>, Option<WinKey>)>,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        let map_shown = self.map_shown;
        let Some(c) = self.cycle.as_mut() else { return };
        if let Some((wins, fg)) = snapshot {
            c.snapshot = wins;
            c.fg = fg;
        }
        // Saltear las que se cerraron.
        let alive: Vec<WinKey> =
            c.order.iter().copied().filter(|w| c.snapshot.iter().any(|s| s.key == *w)).collect();
        if alive.is_empty() {
            return;
        }
        let cur = c.order.get(c.pos).copied();
        let i = cur.and_then(|w| alive.iter().position(|&x| x == w)).unwrap_or(0) as i64;
        let n = alive.len() as i64;
        let next = alive[(i + delta).rem_euclid(n) as usize];
        c.pos = c.order.iter().position(|&w| w == next).unwrap_or(0);
        if !c.list {
            c.list = true;
            c.list_since = now_us;
            if map_shown {
                c.list_shown = true;
            } else {
                c.list_due = Some(now_us + LIST_DELAY_US);
            }
        }
        if c.list_shown {
            self.send_list(out);
        }
    }

    fn send_list(&mut self, out: &mut Vec<Effect>) {
        let active = self.desks.active();
        let Some(c) = self.cycle.as_ref() else { return };
        let rows: Vec<WinInfo> = c
            .order
            .iter()
            .filter_map(|w| c.snapshot.iter().find(|s| s.key == *w).cloned())
            .collect();
        let sel =
            c.order.get(c.pos).and_then(|k| rows.iter().position(|r| r.key == *k)).unwrap_or(0);
        let view = crate::listview::list_view(c.letter, &rows, sel, active);
        out.push(Effect::ShowList { view });
    }

    fn list_deadline(&self) -> Option<u64> {
        let c = self.cycle.as_ref().filter(|c| c.list)?;
        Some(c.list_due.unwrap_or(c.list_since + LIST_MAX_US))
    }

    fn list_tick(&mut self, now_us: u64, out: &mut Vec<Effect>) {
        let Some(c) = self.cycle.as_mut().filter(|c| c.list) else { return };
        if c.list_due.is_some_and(|d| d <= now_us) {
            c.list_due = None;
            c.list_shown = true;
            self.send_list(out);
        } else if now_us >= c.list_since + LIST_MAX_US {
            out.push(Effect::Log("Lista de ventanas: 30 s sin elegir; la cierro.".into()));
            self.drop_cycle(out);
        }
    }

    /// Un clic con la lista a la vista: preguntar qué fila es. `false` si no
    /// hay lista (el clic sigue su camino: arrastrar).
    pub(super) fn list_click(
        &mut self,
        button: crate::mouse::Button,
        at: (i32, i32),
        out: &mut Vec<Effect>,
    ) -> bool {
        if !self.cycle.as_ref().is_some_and(|c| c.list && c.list_shown) {
            return false;
        }
        if button != crate::mouse::Button::Left {
            return true; // otro botón: nada (ni arrastrar)
        }
        let req = self.req();
        self.pending.insert(req, Pending::ListHit);
        out.push(Effect::ListHit { req, x: at.0, y: at.1 });
        true
    }

    /// La fila del clic (`row`: entre las visibles): ir a esa ventana; afuera
    /// de la lista, cerrarla y quedarse donde está.
    fn list_clicked(&mut self, row: Option<usize>, now_us: u64, out: &mut Vec<Effect>) {
        let Some(row) = row else {
            self.drop_cycle(out);
            return;
        };
        let active = self.desks.active();
        let Some(c) = self.cycle.as_mut() else { return };
        let rows: Vec<WinKey> =
            c.order.iter().copied().filter(|w| c.snapshot.iter().any(|s| s.key == *w)).collect();
        let infos: Vec<WinInfo> =
            rows.iter().filter_map(|k| c.snapshot.iter().find(|s| s.key == *k).cloned()).collect();
        let sel = c.order.get(c.pos).and_then(|k| rows.iter().position(|r| r == k)).unwrap_or(0);
        let view = crate::listview::list_view(c.letter, &infos, sel, active);
        let Some(&key) = rows.get(view.above + row) else { return };
        c.pos = c.order.iter().position(|&w| w == key).unwrap_or(c.pos);
        self.commit_list(now_us, out);
    }

    /// Olvidar el ciclo de la letra (y cerrar la lista, si se ve).
    fn drop_cycle(&mut self, out: &mut Vec<Effect>) {
        if let Some(c) = self.cycle.take()
            && c.list_shown
        {
            out.push(Effect::HideList);
        }
    }

    /// Soltar Caps: sin lista, confirma (ya se enfocó); con lista, va a la
    /// elegida.
    fn commit_list(&mut self, now_us: u64, out: &mut Vec<Effect>) {
        let Some(c) = self.cycle.take() else { return };
        if !c.list {
            return;
        }
        if c.list_shown {
            out.push(Effect::HideList);
        }
        let Some(&target) = c.order.get(c.pos) else { return };
        if Some(target) == c.front {
            return;
        }
        if !c.snapshot.iter().any(|w| w.key == target) {
            out.push(Effect::Log("Lista de ventanas: esa ventana ya se cerró.".into()));
            return;
        }
        let app = c
            .snapshot
            .iter()
            .find(|w| w.key == target)
            .map(|w| w.app_id().name().to_owned())
            .unwrap_or_default();
        self.focus_window(c.letter, now_us, target, &app, &c.snapshot, c.fg, now_us, out);
    }

    #[allow(clippy::too_many_arguments)]
    fn focus_window(
        &mut self,
        letter: u8,
        key_us: u64,
        window: WinKey,
        app: &str,
        windows: &[WinInfo],
        foreground: Option<WinKey>,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        if self.elsewhere(window) {
            // En otro escritorio: ir allá y enfocarla, en una transacción.
            let goal = crate::desks::Goal::Window(window);
            self.start_sync_with(goal, key_us, windows, foreground, now_us, out);
            return;
        }
        let info = windows.iter().find(|w| w.key == window);
        let title = info.map_or("", |w| w.title.as_str());
        // El nombre de la app de **esa** ventana (en una letra sin asignar
        // la lista mezcla apps: Nanofy y Notepad).
        let app = info.map_or(app, |w| w.app_id().name());
        let req = self.req();
        let pending = Pending::Focus {
            letter,
            app: app.to_owned(),
            title: title.to_owned(),
            key_us,
            win: window,
        };
        self.pending.insert(req, pending);
        out.push(Effect::Focus { req, win: window });
    }

    /// Se abrió hace poco y todavía no tiene ventana: solo avisar.
    fn guarded(&mut self, app: &str, key: char, now_us: u64, out: &mut Vec<Effect>) -> bool {
        match self.launching.get(app) {
            Some(&until) if until > now_us => {
                out.push(Effect::Log(format!("Caps + {key}: abriendo {app}… (espera)")));
                true
            }
            _ => false,
        }
    }

    fn start_follow(&mut self, letter: u8, now_us: u64) {
        let fg = self.follow_fg.take();
        self.follow = Some(Follow {
            letter,
            launched_us: now_us,
            step: 0,
            fg,
            scanning: false,
            skip: Vec::new(),
        });
    }

    fn follow_deadline(&self) -> Option<u64> {
        let f = self.follow.as_ref().filter(|f| !f.scanning)?;
        FOLLOW_MS.get(f.step).map(|ms| f.launched_us + ms * 1000)
    }

    fn follow_tick(&mut self, now_us: u64, out: &mut Vec<Effect>) {
        if self.follow_deadline().is_none_or(|d| d > now_us) {
            return;
        }
        let req = self.req();
        self.pending.insert(req, Pending::FollowScan);
        if let Some(f) = self.follow.as_mut() {
            f.scanning = true;
            f.step += 1;
        }
        out.push(Effect::Scan { req });
    }

    /// ¿Ya apareció la ventana de la app recién abierta? Si el usuario no
    /// cambió de ventana mientras tanto, se la trae al frente.
    fn follow_scanned(
        &mut self,
        windows: &[WinInfo],
        fg: Option<WinKey>,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        let Some(f) = self.follow.take() else { return };
        let list: Vec<(WinKey, crate::apps::AppId)> =
            windows.iter().map(|w| (w.key, w.app_id())).collect();
        if let Target::Focus { windows: order, app } = resolve(f.letter, &self.bindings, &list)
            && let Some(&window) = order.iter().find(|w| !f.skip.contains(w))
        {
            if fg == f.fg && fg.is_none_or(|g| !order.contains(&g)) {
                self.focus_window(f.letter, f.launched_us, window, &app, windows, fg, now_us, out);
                // Si no se deja traer (pantalla de carga), se sigue esperando.
                let mut next = Follow { scanning: false, ..f };
                next.skip.push(window);
                self.follow_focus = Some((self.next_req, next));
            } else {
                self.launching.remove(&app);
            }
            return;
        }
        if f.step < FOLLOW_MS.len() {
            self.follow = Some(Follow { scanning: false, ..f });
        }
    }
}

#[path = "world_desks.rs"]
mod world_desks;

#[path = "world_drag.rs"]
mod world_drag;

#[path = "world_screens.rs"]
mod world_screens;

#[path = "world_rules.rs"]
mod world_rules;
pub use world_screens::{ScreenStats, Survey, SurveyWin};

/// Índice actual de un monitor (los índices no valen entre observaciones: se
/// identifica por su rectángulo).
fn index_of(monitors: &[MonitorObs], m: MonitorObs) -> Option<usize> {
    monitors.iter().position(|x| x.bounds == m.bounds)
}

/// Corre `r` lo mínimo para que entre en `area` (sin cambiar el tamaño, si
/// entra).
fn fit_inside(r: Rect, area: Rect) -> Rect {
    let dx = if r.left < area.left {
        area.left - r.left
    } else if r.right > area.right {
        (area.right - r.right).max(area.left - r.left)
    } else {
        0
    };
    let dy = if r.top < area.top {
        area.top - r.top
    } else if r.bottom > area.bottom {
        (area.bottom - r.bottom).max(area.top - r.top)
    } else {
        0
    };
    Rect::new(r.left + dx, r.top + dy, r.right + dx, r.bottom + dy)
}

/// La ventana cubre todo su monitor sin estar maximizada: un video o un
/// juego a pantalla completa. Ahí no se mueve nada (la app lo ignoraría).
fn is_fullscreen(obs: &Obs) -> bool {
    !obs.maximized
        && monitor_of(&obs.monitors, obs.frame).is_some_and(|m| obs.frame == obs.monitors[m].bounds)
}

fn is_corner(p: Placement) -> bool {
    matches!(
        p,
        Placement::TopLeft | Placement::TopRight | Placement::BottomLeft | Placement::BottomRight
    )
}

fn next_check_us(j: &Job) -> u64 {
    let ms = CHECKS_MS.get(j.check).copied().unwrap_or(CHECKS_MS[CHECKS_MS.len() - 1]);
    j.placed_us + ms * 1000
}

/// Texto corto de una acción, para el log.
pub fn describe(action: HyperAction) -> String {
    match action {
        HyperAction::Letter { letter, thumb: false } => format!("Caps + {}", letter as char),
        HyperAction::Letter { letter, thumb: true } => format!("Caps + Alt + {}", letter as char),
        HyperAction::Digit { digit, .. } => format!("Caps + {digit}"),
        HyperAction::Escape => "Caps + Esc".to_owned(),
        HyperAction::Other { vk, .. } => format!("Caps + tecla {vk:#04x}"),
    }
}

/// Recorta un texto a `max` caracteres con "…".
pub fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::parse_bindings;

    /// Plataforma de mentira: ventanas fijas; ejecuta los efectos y devuelve
    /// los eventos de respuesta, como haría el brain.
    struct Sim {
        windows: Vec<WinInfo>,
        foreground: Option<WinKey>,
        focus_ok: bool,
        logs: Vec<String>,
        focused: Vec<WinKey>,
        launched: Vec<String>,
        quit: bool,
        /// La lista de ventanas que se ve (plan-apps.md §8).
        list: Option<crate::listview::ListView>,
        list_shows: usize,
        /// La fila que "cae" bajo un clic (`None`: afuera de la lista).
        hit: Option<usize>,
        /// La app que se abrió sigue corriendo (cargando).
        still_running: bool,
        /// Traer al frente esta ventana falla (pantalla de carga).
        unfocusable: Option<WinKey>,
    }

    fn win(hwnd: isize, app: &str) -> WinInfo {
        WinInfo {
            key: WinKey { hwnd, pid: hwnd as u32 + 1000 },
            app: app.into(),
            title: app.into(),
            ..WinInfo::default()
        }
    }

    impl Sim {
        fn new(apps: &[&str]) -> Self {
            let windows: Vec<WinInfo> =
                apps.iter().enumerate().map(|(i, a)| win(i as isize + 1, a)).collect();
            let foreground = windows.first().map(|w| w.key);
            Self {
                windows,
                foreground,
                focus_ok: true,
                logs: Vec::new(),
                focused: Vec::new(),
                launched: Vec::new(),
                quit: false,
                list: None,
                list_shows: 0,
                hit: None,
                still_running: false,
                unfocusable: None,
            }
        }

        /// Corre `event` y todas las respuestas hasta que no quede nada.
        fn run(&mut self, w: &mut World, event: Event) {
            let mut queue = vec![event];
            while let Some(e) = queue.pop() {
                for effect in w.step(e, 1_000_000) {
                    match effect {
                        Effect::Scan { req } => queue.push(Event::Windows {
                            req,
                            result: Ok((self.windows.clone(), self.foreground)),
                        }),
                        Effect::Focus { req, win } => {
                            let ok = self.focus_ok && self.unfocusable != Some(win);
                            if ok {
                                // Como Windows: la enfocada pasa arriba del orden Z.
                                if let Some(i) = self.windows.iter().position(|w| w.key == win) {
                                    let w = self.windows.remove(i);
                                    self.windows.insert(0, w);
                                }
                                self.foreground = Some(win);
                                self.focused.push(win);
                            }
                            let report = FocusReport { ok, neutral_key: false, elapsed_us: 1500 };
                            queue.push(Event::Focused { req, result: Ok(report) });
                        }
                        Effect::Launch { req, path, only_if_closed, .. } => {
                            if only_if_closed && self.still_running {
                                queue.push(Event::Launched {
                                    req,
                                    result: Err(format!("{LOADING}: sigue corriendo")),
                                });
                                continue;
                            }
                            self.launched.push(path);
                            queue.push(Event::Launched { req, result: Ok(()) });
                        }
                        Effect::Revive { req, names } => {
                            self.launched.push(format!("(proceso) {}", names.join(",")));
                            queue.push(Event::Launched { req, result: Ok(()) });
                        }
                        Effect::Log(line) => self.logs.push(line),
                        Effect::Quit => self.quit = true,
                        Effect::SetDesks { req, items } => {
                            for (k, n) in items {
                                if let Some(w) = self.windows.iter_mut().find(|w| w.key == k) {
                                    w.desk = Some(n);
                                }
                            }
                            queue.push(Event::DesksSet { req, failed: Vec::new() });
                        }
                        Effect::Show { .. } | Effect::Hide { .. } | Effect::FocusShell { .. } => {
                            panic!("todo está en el escritorio 1")
                        }
                        Effect::Notice(_) | Effect::MovePointer { .. } => {}
                        // Sin pantallas en esta simulación: el Survey queda sin
                        // respuesta y no se rescata nada.
                        Effect::Survey { .. } | Effect::ScreensChanged => {}
                        Effect::ShowList { view } => {
                            self.list = Some(view);
                            self.list_shows += 1;
                        }
                        Effect::HideList => self.list = None,
                        Effect::ListHit { req, .. } => {
                            queue.push(Event::ListHit { req, row: self.hit })
                        }
                        Effect::Rescue { .. } => panic!("sin pantallas no hay rescate"),
                        Effect::SaveRule { .. } => panic!("la capa de apps no guarda reglas"),
                        Effect::InspectAt { .. }
                        | Effect::Raise { .. }
                        | Effect::DragStart { .. }
                        | Effect::DragStop { .. } => panic!("la capa de apps no arrastra"),
                        Effect::Inspect { .. }
                        | Effect::Place { .. }
                        | Effect::Fly { .. }
                        | Effect::Land { .. }
                        | Effect::ShowMap { .. }
                        | Effect::MapKey { .. }
                        | Effect::MapLayer { .. }
                        | Effect::HideMap
                        | Effect::ResetKeyboard
                        | Effect::SetPaused(_)
                        | Effect::Normalize { .. }
                        | Effect::Maximize { .. } => {
                            panic!("la capa de apps no coloca ventanas")
                        }
                    }
                }
            }
        }
    }

    fn letter(l: u8) -> Event {
        Event::Action {
            action: HyperAction::Letter { letter: l, thumb: false },
            held: 0,
            age_ms: 2,
        }
    }

    #[test]
    fn letter_scans_then_focuses_the_app() {
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new(&["code", "brave", "slack"]);
        let brave = sim.windows[1].key;
        sim.run(&mut w, letter(b'B'));
        assert_eq!(sim.focused, vec![brave]);
        assert_eq!(w.activations(), Activations { total: 1, ok: 1, neutral_key: 0 });
        assert!(sim.logs[0].starts_with("Caps + B → brave «brave»: ok"), "{:?}", sim.logs);
        assert_eq!(w.in_flight(), 0, "no quedan pedidos colgados");
    }

    #[test]
    fn repeating_the_letter_cycles_the_windows_of_the_app() {
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new(&["brave", "code", "brave"]);
        let (first, second) = (sim.windows[0].key, sim.windows[2].key);
        sim.run(&mut w, letter(b'B')); // la primera ya está al frente → la otra
        sim.run(&mut w, letter(b'B')); // y vuelve
        assert_eq!(sim.focused, vec![second, first]);
    }

    #[test]
    fn bound_but_closed_app_is_launched() {
        let (b, _) = parse_bindings("n = notion | C:\\Notion.exe");
        let mut w = World::new(b);
        let mut sim = Sim::new(&["code"]);
        sim.run(&mut w, letter(b'N'));
        assert_eq!(sim.launched, vec!["C:\\Notion.exe".to_owned()]);
        assert!(sim.logs[0].contains("abriendo notion"));
    }

    fn layer_off() -> Event {
        Event::Layer(crate::keyboard::Layer::Off)
    }

    fn layer_apps() -> Event {
        Event::Layer(crate::keyboard::Layer::Apps)
    }

    fn key(vk: u8) -> Event {
        Event::Action { action: HyperAction::Other { vk, thumb: false }, held: 0, age_ms: 2 }
    }

    /// Con hilo ui (la lista se puede dibujar) y Caps abajo.
    fn with_list(apps: &[&str]) -> (World, Sim) {
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new(apps);
        sim.run(&mut w, Event::MapEnabled(true));
        sim.run(&mut w, layer_apps());
        (w, sim)
    }

    #[test]
    fn with_three_windows_the_second_tap_opens_the_list_and_release_goes() {
        let (mut w, mut sim) = with_list(&["code", "brave", "brave", "brave"]);
        let (b1, b2, b3) = (sim.windows[1].key, sim.windows[2].key, sim.windows[3].key);
        sim.run(&mut w, letter(b'B')); // primer toque: inmediato
        assert_eq!(sim.focused, vec![b1]);
        sim.run(&mut w, letter(b'B')); // segundo: lista, sin enfocar
        sim.run(&mut w, letter(b'B'));
        assert_eq!(sim.focused, vec![b1], "en modo lista no se enfoca nada");
        assert!(sim.list.is_none(), "la lista espera 150 ms");
        let effects = w.step(Event::Tick, 1_000_000 + 200_000);
        let Some(Effect::ShowList { view }) =
            effects.iter().find(|e| matches!(e, Effect::ShowList { .. }))
        else {
            panic!("{effects:?}")
        };
        assert_eq!(view.count, 3);
        assert_eq!(view.selected, 2, "dos toques después del primero: la tercera");
        sim.run(&mut w, layer_off()); // soltar: va a la elegida
        assert_eq!(sim.focused, vec![b1, b3]);
        let _ = b2;
    }

    fn click() -> Event {
        Event::Pointer { button: crate::mouse::Button::Left, down: true, x: 500, y: 400 }
    }

    fn open_list(w: &mut World, sim: &mut Sim) {
        sim.run(w, letter(b'B'));
        sim.run(w, letter(b'B'));
        let effects = w.step(Event::Tick, 1_000_000 + 200_000);
        for e in effects {
            if let Effect::ShowList { view } = e {
                sim.list = Some(view);
            }
        }
        assert!(sim.list.is_some(), "la lista a la vista");
    }

    #[test]
    fn a_click_on_a_row_goes_to_that_window() {
        let (mut w, mut sim) = with_list(&["code", "brave", "brave", "brave"]);
        let (b1, b3) = (sim.windows[1].key, sim.windows[3].key);
        open_list(&mut w, &mut sim);
        sim.hit = Some(2);
        sim.run(&mut w, click());
        assert_eq!(sim.focused, vec![b1, b3], "{:?}", sim.logs);
        assert!(sim.list.is_none());
        // Soltar Caps después no vuelve a enfocar nada.
        sim.run(&mut w, layer_off());
        assert_eq!(sim.focused.len(), 2);
    }

    #[test]
    fn a_click_outside_the_list_closes_it_and_stays() {
        let (mut w, mut sim) = with_list(&["code", "brave", "brave", "brave"]);
        let b1 = sim.windows[1].key;
        open_list(&mut w, &mut sim);
        sim.hit = None;
        sim.run(&mut w, click());
        assert!(sim.list.is_none());
        sim.run(&mut w, layer_off());
        assert_eq!(sim.focused, vec![b1], "se queda donde estaba");
    }

    #[test]
    fn arrows_move_and_esc_stays_where_you_are() {
        let (mut w, mut sim) = with_list(&["code", "brave", "brave", "brave"]);
        let b1 = sim.windows[1].key;
        sim.run(&mut w, letter(b'B'));
        sim.run(&mut w, letter(b'B'));
        sim.run(&mut w, key(0x28)); // ↓
        sim.run(&mut w, key(0x26)); // ↑
        sim.run(&mut w, Event::Action { action: HyperAction::Escape, held: 0, age_ms: 2 });
        assert!(!sim.quit);
        sim.run(&mut w, layer_off());
        assert_eq!(sim.focused, vec![b1], "Esc: se queda en la del primer toque");
        assert!(sim.list.is_none());
    }

    #[test]
    fn two_windows_still_alternate_instantly() {
        let (mut w, mut sim) = with_list(&["brave", "code", "brave"]);
        let (b1, b2) = (sim.windows[0].key, sim.windows[2].key);
        sim.run(&mut w, letter(b'B'));
        sim.run(&mut w, letter(b'B'));
        assert_eq!(sim.focused, vec![b2, b1]);
        assert_eq!(sim.list_shows, 0);
    }

    #[test]
    fn releasing_quickly_goes_without_drawing_the_list() {
        let (mut w, mut sim) = with_list(&["code", "brave", "brave", "brave"]);
        let (b1, b2) = (sim.windows[1].key, sim.windows[2].key);
        sim.run(&mut w, letter(b'B'));
        sim.run(&mut w, letter(b'B'));
        sim.run(&mut w, layer_off());
        assert_eq!(sim.focused, vec![b1, b2]);
        assert_eq!(sim.list_shows, 0, "nunca se dibujó");
    }

    #[test]
    fn another_letter_closes_the_list_and_goes_there() {
        let (mut w, mut sim) = with_list(&["code", "brave", "brave", "brave"]);
        let code = sim.windows[0].key;
        sim.run(&mut w, letter(b'B'));
        sim.run(&mut w, letter(b'B'));
        sim.run(&mut w, letter(b'C'));
        assert_eq!(sim.focused.last(), Some(&code));
        assert!(sim.list.is_none());
    }

    #[test]
    fn the_letter_works_like_alt_tab() {
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new(&["brave", "code", "brave", "brave"]);
        let (b2, b3) = (sim.windows[2].key, sim.windows[3].key);
        sim.run(&mut w, layer_apps()); // Caps abajo (sin hilo ui: sin lista)
        sim.run(&mut w, letter(b'B')); // ya en una de Brave: la anterior
        sim.run(&mut w, letter(b'B')); // sin soltar Caps: la tercera
        assert_eq!(sim.focused, vec![b2, b3]);
        sim.run(&mut w, layer_off()); // soltar confirma
        sim.run(&mut w, letter(b'B')); // de nuevo: vuelve a la anterior
        assert_eq!(sim.focused, vec![b2, b3, b2]);
    }

    #[test]
    fn unbound_letter_mixes_every_app_with_that_initial() {
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new(&["nanofy", "notepad"]);
        let notepad = sim.windows[1].key;
        sim.run(&mut w, letter(b'N'));
        assert_eq!(sim.focused, vec![notepad], "Notepad ya no queda detrás de Nanofy");
    }

    #[test]
    fn the_only_window_says_so() {
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new(&["brave", "code"]);
        sim.run(&mut w, letter(b'B'));
        assert!(sim.focused.is_empty());
        assert!(sim.logs[0].contains("brave es la única ventana"), "{:?}", sim.logs);
        assert!(crate::notices::notice_for(&sim.logs[0]).is_some());
    }

    #[test]
    fn a_bound_app_in_the_tray_is_opened_again_once() {
        let (b, _) = parse_bindings("s = steam");
        let mut w = World::new(b);
        let mut sim = Sim::new(&["code"]);
        sim.run(&mut w, letter(b'S'));
        assert_eq!(sim.launched, vec!["(proceso) steam".to_owned()]);
        sim.run(&mut w, letter(b'S'));
        assert_eq!(sim.launched.len(), 1, "no la abre dos veces: {:?}", sim.logs);
        assert!(sim.logs.last().unwrap().contains("abriendo steam… (espera)"), "{:?}", sim.logs);
        // Aparece su ventana (de steamwebhelper, familia steam): se enfoca.
        let mut steam = win(9, "steamwebhelper");
        steam.family = Some("steam".into());
        sim.windows.push(steam.clone());
        let effects = w.step(Event::Tick, 1_000_000 + 300_000);
        let Some(Effect::Scan { req }) = effects.first().cloned() else { panic!("{effects:?}") };
        let result = Ok((sim.windows.clone(), sim.foreground));
        sim.run(&mut w, Event::Windows { req, result });
        assert_eq!(sim.focused, vec![steam.key], "{:?}", sim.logs);
    }

    #[test]
    fn the_map_says_where_the_next_tap_goes() {
        // Prueba del usuario: el mapa decía "notepad" y la tecla iba a Nanofy.
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new(&["claude", "notepad", "nanofy"]);
        w.map_wins = sim.windows.clone();
        w.map_fg = sim.foreground;
        assert_eq!(w.predict(b'N').map(|(a, _)| a).as_deref(), Some("notepad"));
        sim.run(&mut w, letter(b'N')); // va a Notepad; Caps sigue abajo
        assert!(sim.logs.last().unwrap().contains("→ notepad «notepad»"), "{:?}", sim.logs);
        assert_eq!(
            w.predict(b'N').map(|(a, _)| a).as_deref(),
            Some("nanofy"),
            "el próximo toque va a Nanofy"
        );
        sim.run(&mut w, letter(b'N'));
        assert!(sim.logs.last().unwrap().contains("→ nanofy «nanofy»"), "{:?}", sim.logs);
        sim.run(&mut w, layer_off());
        assert_eq!(
            w.predict(b'N').map(|(a, _)| a).as_deref(),
            Some("notepad"),
            "soltado: la anterior"
        );
    }

    /// Avanza el reloj hasta `at` (µs) y contesta el `Scan` del seguimiento.
    fn follow_until(w: &mut World, sim: &mut Sim, at: u64) {
        let effects = w.step(Event::Tick, at);
        if let Some(Effect::Scan { req }) = effects.first().cloned() {
            let result = Ok((sim.windows.clone(), sim.foreground));
            sim.run(w, Event::Windows { req, result });
        }
    }

    #[test]
    fn a_splash_that_cannot_be_focused_does_not_stop_the_wait() {
        let (b, _) = parse_bindings("i = idea64 | C:/wm-test/idea64.exe");
        let mut w = World::new(b);
        let mut sim = Sim::new(&["code"]);
        sim.run(&mut w, letter(b'I'));
        assert_eq!(sim.launched.len(), 1);
        // Aparece la pantalla de carga: no se deja traer al frente.
        let splash = win(20, "idea64");
        sim.unfocusable = Some(splash.key);
        sim.windows.push(splash.clone());
        follow_until(&mut w, &mut sim, 1_000_000 + 300_000);
        assert!(sim.focused.is_empty());
        // Después, la ventana de verdad: se trae al frente sola.
        let main = win(21, "idea64");
        sim.windows.push(main.clone());
        follow_until(&mut w, &mut sim, 1_000_000 + 800_000);
        assert_eq!(sim.focused, vec![main.key], "{:?}", sim.logs);
    }

    #[test]
    fn pressing_again_while_waiting_reopens_only_if_it_is_not_running() {
        let (b, _) = parse_bindings("i = idea64 | C:/wm-test/idea64.exe");
        let mut w = World::new(b);
        let mut sim = Sim::new(&["code"]);
        sim.run(&mut w, letter(b'I'));
        sim.still_running = true; // cargando
        sim.run(&mut w, letter(b'I'));
        assert_eq!(sim.launched.len(), 1, "{:?}", sim.logs);
        assert!(sim.logs.last().unwrap().contains("abriendo idea64… (espera)"), "{:?}", sim.logs);
        sim.still_running = false; // no llegó a abrir (la cerraron, se negó)
        sim.run(&mut w, letter(b'I'));
        assert_eq!(sim.launched.len(), 2, "se abre otra vez: {:?}", sim.logs);
    }

    #[test]
    fn unknown_letter_only_logs() {
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new(&["code"]);
        sim.run(&mut w, letter(b'Q'));
        assert!(sim.focused.is_empty());
        assert_eq!(sim.logs, vec!["Caps + Q: no hay ninguna app abierta con la Q.".to_owned()]);
    }

    #[test]
    fn late_actions_are_discarded() {
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new(&["brave"]);
        let late = Event::Action {
            action: HyperAction::Letter { letter: b'B', thumb: false },
            held: 0,
            age_ms: MAX_ACTION_AGE_MS + 1,
        };
        sim.run(&mut w, late);
        assert!(sim.focused.is_empty());
        assert!(sim.logs[0].starts_with("Acción descartada"));
    }

    #[test]
    fn failed_focus_is_counted() {
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new(&["code", "brave"]);
        sim.focus_ok = false;
        sim.run(&mut w, letter(b'B'));
        assert_eq!(w.activations(), Activations { total: 1, ok: 0, neutral_key: 0 });
        assert!(sim.logs[0].contains("FALLÓ"));
    }

    #[test]
    fn answers_to_unknown_requests_are_ignored() {
        let mut w = World::new(Vec::new());
        let stray = Event::Focused { req: 99, result: Err("x".into()) };
        assert!(w.step(stray, 0).is_empty());
    }

    #[test]
    fn escape_quits_and_layers_are_tracked() {
        let mut w = World::new(Vec::new());
        w.set_esc_quits(true); // modo prueba
        let mut sim = Sim::new(&[]);
        sim.run(&mut w, Event::Layer(Layer::Window));
        assert_eq!(w.layer(), Layer::Window);
        sim.run(&mut w, Event::KeyboardReset);
        assert_eq!(w.layer(), Layer::Off);
        sim.run(&mut w, Event::Action { action: HyperAction::Escape, held: 0, age_ms: 0 });
        assert!(sim.quit);
    }

    #[test]
    fn latency_is_measured_from_the_key() {
        let mut w = World::new(Vec::new());
        let effects = w.step(letter(b'B'), 10_000); // tecla 2 ms antes: 8000 µs
        let Effect::Scan { req } = effects[0] else { panic!("{effects:?}") };
        let win = win(1, "brave");
        let effects = w.step(Event::Windows { req, result: Ok((vec![win.clone()], None)) }, 11_000);
        // Antes, la adopción anota el escritorio en la ventana (hito 4).
        let Some(Effect::Focus { req, .. }) =
            effects.iter().find(|e| matches!(e, Effect::Focus { .. })).cloned()
        else {
            panic!("{effects:?}")
        };
        let report = FocusReport { ok: true, neutral_key: false, elapsed_us: 500 };
        let effects = w.step(Event::Focused { req, result: Ok(report) }, 13_000);
        let Effect::Log(line) = &effects[0] else { panic!("{effects:?}") };
        assert!(line.contains("desde la tecla 5.0 ms"), "{line}");
    }
}

#[cfg(test)]
mod fullscreen_tests {
    use super::*;
    use crate::place::MonitorObs;

    fn obs(frame: Rect) -> Obs {
        let m = MonitorObs {
            bounds: Rect::new(0, 0, 1920, 1080),
            work: Rect::new(0, 0, 1920, 1032),
            dpi: 96,
        };
        Obs {
            key: WinKey { hwnd: 1, pid: 1 },
            app: "brave".into(),
            title: String::new(),
            frame,
            outer: frame,
            dpi: 96,
            maximized: false,
            minimized: false,
            monitors: vec![m],
            no_fly: None,
            pointer: None,
        }
    }

    #[test]
    fn c_f_and_z_leave_a_fullscreen_window_alone() {
        // Prueba del usuario: C sobre un video de Brave a pantalla completa
        // pedía moverlo y terminaba en "la app no responde".
        let full = obs(Rect::new(0, 0, 1920, 1080));
        let mut w = World::new(Vec::new());
        let mut out = Vec::new();
        w.center(full.clone(), "C".into(), 0, 0, &mut out);
        w.full(full.clone(), "F".into(), 0, 0, &mut out);
        assert!(
            out.iter().all(|e| matches!(e, Effect::Log(l) if l.contains("pantalla completa"))),
            "{out:?}"
        );
        assert_eq!(out.len(), 2);
        // Z: la entrada no se pierde.
        let entry = Entry {
            win: full.key,
            frame: Rect::new(0, 0, 800, 600),
            maximized: false,
            tag: 1,
            desk: None,
            went: false,
        };
        let mut out = Vec::new();
        w.undo(entry, full, 0, 0, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(w.journal.len(), 1, "queda para deshacer después");
    }
}

/// Una orden del menú de la bandeja que decide el modelo. Abrir los ajustes e
/// iniciar con Windows los resuelve el hilo de la bandeja (son E/S).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Quit,
    SetPaused(bool),
    Undo,
    /// "Mostrar todas las ventanas": todas al escritorio activo (hito 4c).
    ShowAll,
    /// Explorer se reinició: revisar que cada ventana esté como pide su
    /// escritorio.
    Reconcile,
    /// Ir a este escritorio (menú de la bandeja, hito 5e).
    Desk(u8),
    /// v1.2: "Abrir siempre así" (bandeja): guardar cómo está `win` como
    /// la regla "al abrir" de la app de `letter` (plan-v1.2.md §4).
    OpenLikeThis {
        win: WinKey,
        letter: u8,
    },
}

#[cfg(test)]
mod command_tests {
    use super::*;
    use crate::keyboard::{HyperAction, Layer};

    fn letter(l: u8, thumb: bool) -> Event {
        Event::Action { action: HyperAction::Letter { letter: l, thumb }, held: 0, age_ms: 0 }
    }

    #[test]
    fn paused_ignores_keys_and_layers() {
        let mut w = World::new(Vec::new());
        w.step(Event::MapEnabled(true), 0);
        let out = w.step(Event::Command(Command::SetPaused(true)), 0);
        assert!(out.contains(&Effect::SetPaused(true)), "{out:?}");
        assert!(w.paused());
        for (i, e) in [
            Event::Layer(Layer::Apps),
            letter(b'B', false),
            Event::Layer(Layer::Window),
            letter(b'A', true),
            Event::Action { action: HyperAction::Escape, held: 0, age_ms: 0 },
        ]
        .into_iter()
        .enumerate()
        {
            assert!(w.step(e, i as u64 * 1000).is_empty(), "en pausa no hace nada");
        }
        assert_eq!(w.deadline(), None, "ni siquiera arma el mapa");
        let out = w.step(Event::Command(Command::SetPaused(false)), 10_000);
        assert!(out.contains(&Effect::SetPaused(false)));
        assert!(!w.step(letter(b'B', false), 11_000).is_empty(), "reanudado, vuelve a actuar");
    }

    #[test]
    fn pausing_with_the_map_visible_hides_it() {
        let mut w = World::new(Vec::new());
        w.step(Event::MapEnabled(true), 0);
        w.step(Event::Layer(Layer::Apps), 0);
        let out = w.step(Event::Tick, 700_000);
        let Some(Effect::Scan { req }) = out.first().cloned() else { panic!("{out:?}") };
        w.step(Event::Windows { req, result: Ok((Vec::new(), None)) }, 700_000);
        assert!(w.map_visible());
        let out = w.step(Event::Command(Command::SetPaused(true)), 800_000);
        assert!(out.contains(&Effect::HideMap), "{out:?}");
        assert!(!w.map_visible());
    }

    #[test]
    fn quit_from_the_tray_and_escape_only_in_test_mode() {
        let mut w = World::new(Vec::new());
        assert_eq!(w.step(Event::Command(Command::Quit), 0), vec![Effect::Quit]);
        let esc = || Event::Action { action: HyperAction::Escape, held: 0, age_ms: 0 };
        let mut w = World::new(Vec::new());
        assert!(!w.step(esc(), 0).contains(&Effect::Quit), "Caps + Esc ya no sale");
        w.set_esc_quits(true);
        assert!(w.step(esc(), 0).contains(&Effect::Quit), "en modo prueba sí");
    }

    #[test]
    fn undo_from_the_tray_is_like_z() {
        let mut w = World::new(Vec::new());
        assert!(!w.can_undo());
        let out = w.step(Event::Command(Command::Undo), 0);
        assert!(
            matches!(&out[..], [Effect::Log(l), Effect::Notice(_)] if l.contains("nada para deshacer")),
            "{out:?}"
        );
    }
}
