import { DownloadCta } from '@/components/download-cta';
import { Features } from '@/components/features';
import { Hero } from '@/components/hero';
import { Hotkeys } from '@/components/hotkeys';
import { InterfaceSection } from '@/components/interface-section';
import { Privacy } from '@/components/privacy';
import { Providers } from '@/components/providers';
import { SiteFooter } from '@/components/site-footer';
import { TopNav } from '@/components/top-nav';

const Home = () => {
  return (
    <>
      <TopNav />
      <main id='content'>
        <Hero />
        <Features />
        <Privacy />
        <InterfaceSection />
        <Hotkeys />
        <Providers />
        <DownloadCta />
      </main>
      <SiteFooter />
    </>
  );
};

export default Home;
