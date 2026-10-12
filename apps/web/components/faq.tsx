import { ChevronDownIcon } from '@marvis/ui';
import { FAQS } from '@/lib/faq';
import { leadTop, sectionStack, surfaceBg } from './styles';

export const Faq = () => (
  <section
    className='section'
    id='faq'
    data-od-id='faq'
    style={surfaceBg}
    aria-labelledby='faq-heading'>
    <div
      className='container stack'
      style={sectionStack}>
      <div style={{ maxWidth: '44ch' }}>
        <p className='eyebrow'>Common Questions</p>
        <h2 id='faq-heading'>Frequently asked questions</h2>
        <p
          className='lead'
          style={leadTop}>
          Everything you need to know about privacy, architecture, offline local
          models, and how Marvis works on your machine.
        </p>
      </div>

      <div
        className='faq-list'
        style={{ maxWidth: '820px' }}>
        {FAQS.map((faq, index) => (
          <details
            key={faq.question}
            className='faq-item'
            open={index === 0}>
            <summary className='faq-summary'>
              <span>{faq.question}</span>
              <ChevronDownIcon className='faq-icon' />
            </summary>
            <div className='faq-answer'>
              <p>{faq.answer}</p>
            </div>
          </details>
        ))}
      </div>
    </div>
  </section>
);
