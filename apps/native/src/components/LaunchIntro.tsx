/**
 * Launch wordmark — the idle capsule's once-per-launch beat, in two legs:
 *
 *  1. CSS leg (index.html): `#boot-splash` — a sibling of #root, so
 *     React never replaces it — paints the mark + Galada wordmark on the
 *     webview's first frame and fades the letters in via stylesheet
 *     animations. This covers the JS-boot gap (no white flash, brand is
 *     up before React mounts).
 *  2. GSAP leg (here): on mount the timeline snapshots each element's
 *     current animated opacity, kills the CSS animations (inline
 *     `animation: none` also blocks the `.ready` fallback landing late),
 *     restores the snapshot, and finishes the run — hold, letters
 *     dissolve, mark slides over and condenses onto the iris button,
 *     remaining icons stagger in. The handoff continues mid-flight
 *     instead of restarting, so the takeover is invisible.
 *
 * Mounted by Bar only while the pill is collapsed and inert
 * (`!introDone && !showInputRow`), covering the `gate === null` boot
 * frame too. An interrupt (hotkey expand, gate flip to
 * `needs_permission`, boot error) unmounts it mid-flight; the context
 * revert restores the row and Bar removes the splash node — an
 * interrupted intro never replays (a remount finds no splash and
 * completes immediately), so it finishes once per webview.
 * Reduced-motion never mounts it (`introDone` initializes true); the
 * splash then shows the static lockup until Bar's effect removes it.
 */
import gsap from 'gsap';
import { useGSAP } from '@gsap/react';

gsap.registerPlugin(useGSAP);

export const LaunchIntro = ({
  onDone,
}: {
  /** Timeline end — flips `introDone`, which unmounts this component and
   *  lets Bar remove the (already dissolved) splash node. */
  onDone: () => void;
}) => {
  useGSAP((_, contextSafe) => {
    const splash = document.getElementById('boot-splash');
    // The collapsed row's controls — the form only renders in the
    // main/null-gate capsule, so this can't catch gate-card or retry
    // buttons. The iris is found structurally (the only `aria-hidden`
    // span inside a bar control — the icon buttons' children are svgs),
    // so a reordered or added control can't silently shift the merge
    // target or hide a button forever.
    const icons = document.querySelectorAll('form button');
    const iris = document.querySelector<HTMLElement>(
      'form button > span[aria-hidden]',
    );
    const irisBtn = iris?.closest('button') ?? null;
    const mark = splash?.querySelector('.boot-mark') ?? null;
    const letters = splash?.querySelectorAll('.boot-letter') ?? [];
    if (
      !splash ||
      !mark ||
      !iris ||
      !irisBtn ||
      letters.length === 0 ||
      icons.length === 0
    ) {
      onDone();
      return;
    }
    // Hide the controls from the first paint — a layout effect runs
    // before the commit flushes, so the row never flashes underneath.
    // Opacity ONLY: a transform here would corrupt the iris rect the
    // merge measures in play() (the icon rises into place on reveal).
    gsap.set(icons, { autoAlpha: 0 });

    const play = (contextSafe ?? ((f: () => void) => f))(() => {
      const els = [mark, ...letters];
      const seen = els.map(
        (el) => parseFloat(getComputedStyle(el).opacity) || 0,
      );
      gsap.set(els, { animation: 'none', opacity: (i) => seen[i] });

      // Merge geometry, measured while the mark is untransformed: box
      // center onto the iris box center, sized to match — eye-to-pupil
      // alignment reads as drifting past the target.
      const m = mark.getBoundingClientRect();
      const i = iris.getBoundingClientRect();
      const scale = i.width / m.width;
      const dx = i.left + i.width / 2 - (m.left + m.width / 2);
      const dy = i.top + i.height / 2 - (m.top + m.height / 2);

      const tl = gsap.timeline({ onComplete: onDone });
      // Cold start (React beat the CSS leg): the mark still pops in.
      // Warm start: it just finishes its fade.
      if (seen[0] < 0.15) {
        tl.fromTo(
          mark,
          { scale: 0.55 },
          { scale: 1, opacity: 1, duration: 0.5, ease: 'back.out(1.8)' },
        );
      } else {
        tl.to(mark, { opacity: 1, duration: 0.2 });
      }
      tl.to(
        letters,
        { opacity: 1, duration: 0.32, stagger: 0.05, ease: 'power1.out' },
        '<0.05',
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
        .fromTo(
          irisBtn,
          { autoAlpha: 0, y: 7 },
          { autoAlpha: 1, y: 0, duration: 0.26 },
          '<0.02',
        )
        .fromTo(
          [...icons].filter((b) => b !== irisBtn),
          { autoAlpha: 0, y: 7 },
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

    // Galada is preloaded, but a cold cache can still lag — start behind
    // the font rather than continuing in the fallback cursive. A failed
    // load plays anyway.
    if (document.fonts.check('24px Galada')) {
      play();
    } else {
      void document.fonts.load('24px Galada').then(play, play);
    }
  });

  return null;
};
