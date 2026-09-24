/**
 * Launch wordmark — the idle capsule's once-per-launch beat: the Marvis
 * mark pops in, the "Marvis" wordmark writes on letter by letter in
 * Galada, holds, then the letters dissolve while the mark slides over
 * and condenses into the capsule's iris button — the brand lockup
 * literally merges into the bar's presence mark. The remaining icons
 * stagger in behind it.
 *
 * Mounted by Bar only while the pill is collapsed and inert
 * (`!introDone && !showInputRow`), so it also covers the `gate === null`
 * boot frame — the icons are hidden from the first paint. An interrupt
 * (hotkey expand, gate flip to `needs_permission`, boot error) unmounts
 * it mid-flight; the context revert restores the row and the intro
 * simply replays on the next idle frame, completing once per webview.
 * Reduced-motion never mounts it (`introDone` initializes true).
 */
import { useRef } from 'react';
import gsap from 'gsap';
import { useGSAP } from '@gsap/react';

gsap.registerPlugin(useGSAP);

const LETTERS = [...'Marvis'];
/** The mark's slate circle (its "eye") sits at cy 17.5 of the 48px
 *  viewBox — the merge aligns THAT point onto the iris pupil, not the
 *  box center. */
const MARK_EYE_Y = 17.5 / 48;

export const LaunchIntro = ({
  onDone,
}: {
  /** Timeline end — flips `introDone`, which unmounts this overlay and
   *  reverts the icon styles back to their natural (visible) state. */
  onDone: () => void;
}) => {
  const overlayRef = useRef<HTMLDivElement>(null);

  useGSAP(
    (_, contextSafe) => {
      const overlay = overlayRef.current;
      // The overlay is a direct child of the `group/bar` capsule —
      // `overlay.parentElement` resolves it here, where a passed-in
      // ancestor ref would still be null (a child's layout effect runs
      // before the parent div's ref attach in the same commit).
      const root = overlay?.parentElement ?? null;
      if (!overlay || !root || !root.classList.contains('group/bar')) {
        return;
      }
      // The collapsed row's three controls — the form only renders in
      // the main/null-gate capsule, so this can't catch gate-card or
      // retry buttons. icons[0] is the iris button; its inner span is
      // the 24px iris box the mark merges onto.
      const icons = root.querySelectorAll('form button');
      const iris = icons[0]?.querySelector('span');
      const mark = overlay.querySelector('.boot-mark');
      const letters = overlay.querySelectorAll('.boot-letter');
      if (!iris || !mark || !letters.length || !icons.length) {
        return;
      }

      const play = (contextSafe ?? ((f: () => void) => f))(() => {
        // Measure BEFORE the timeline touches anything (transforms are
        // still clean): where the mark's eye must land — the iris box
        // center — and how far it shrinks to become the iris.
        const m = mark.getBoundingClientRect();
        const i = iris.getBoundingClientRect();
        const scale = i.width / m.width;
        const eyeOffsetY = m.height * (MARK_EYE_Y - 0.5);
        const dx = i.left + i.width / 2 - (m.left + m.width / 2);
        const dy =
          i.top + i.height / 2 - (m.top + m.height / 2 + eyeOffsetY * scale);

        gsap
          .timeline({ onComplete: onDone })
          .set(icons, { autoAlpha: 0, y: 7 })
          .fromTo(
            mark,
            { autoAlpha: 0, scale: 0.4 },
            { autoAlpha: 1, scale: 1, duration: 0.5, ease: 'back.out(1.8)' },
          )
          .fromTo(
            letters,
            { autoAlpha: 0, y: '0.45em', rotate: 7, filter: 'blur(5px)' },
            {
              autoAlpha: 1,
              y: 0,
              rotate: 0,
              filter: 'blur(0px)',
              duration: 0.55,
              stagger: 0.055,
              ease: 'power3.out',
            },
            '-=0.3',
          )
          // hold the finished lockup
          .addLabel('dissolve', '+=0.55')
          // letters dissolve upward while the mark slides to the iris
          .to(
            letters,
            {
              autoAlpha: 0,
              y: -6,
              duration: 0.24,
              stagger: 0.03,
              ease: 'power2.in',
            },
            'dissolve',
          )
          .to(
            mark,
            { x: dx, y: dy, scale, duration: 0.5, ease: 'power3.inOut' },
            'dissolve',
          )
          // merge: the mark dissolves into the iris as it lands
          .to(mark, { autoAlpha: 0, duration: 0.18, ease: 'power1.in' })
          .to(icons[0], { autoAlpha: 1, y: 0, duration: 0.26 }, '<0.02')
          .to(
            [icons[1], icons[2]],
            {
              autoAlpha: 1,
              y: 0,
              duration: 0.32,
              stagger: 0.06,
              ease: 'power3.out',
            },
            '<0.06',
          );
      });

      // Galada is preloaded in index.html, but a cold cache can still
      // lag the first paint — start behind the font rather than writing
      // on in the fallback cursive. A failed load plays anyway.
      if (document.fonts.check('26px Galada')) {
        play();
      } else {
        void document.fonts.load('26px Galada').then(play, play);
      }
    },
    { scope: overlayRef },
  );

  return (
    <div
      ref={overlayRef}
      aria-hidden='true'
      className='pointer-events-none absolute inset-0 z-10 flex items-center justify-center gap-1.75'>
      {/* Galada's line box sits high — the wordmark's translate optically
          centers it in the 64px capsule, and the mark rides slightly
          above box-center so it shares the text's visual centerline.
          Both stay invisible until the timeline's fromTo takes over. */}
      <img
        src='/marvis-mark.svg'
        alt=''
        draggable={false}
        className='boot-mark size-5.5 -translate-y-px opacity-0'
      />
      <span className='translate-y-0.5 font-wordmark text-[24px] leading-none text-fg-2'>
        {LETTERS.map((letter, i) => (
          <span
            key={i}
            className='boot-letter inline-block opacity-0'>
            {letter}
          </span>
        ))}
      </span>
    </div>
  );
};
