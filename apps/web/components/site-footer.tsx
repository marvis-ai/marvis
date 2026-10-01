import { Mark } from './icons';

export const SiteFooter = () => (
  <footer
    className='pagefoot'
    data-od-id='footer'>
    <div className='container row-between'>
      <span
        className='row'
        style={{ gap: '10px' }}>
        <Mark
          width={20}
          height={20}
        />
        <span className='wordmark'>Marvis</span>
      </span>
      <span>© 2026 Marvis AI LLC · MIT license</span>
      <span className='meta'>Built with ❤️</span>
    </div>
  </footer>
);
