import React from 'react';
import ReactDOM from 'react-dom/client';
import './index.css';
import { createBrowserRouter, Outlet, RouterProvider } from 'react-router-dom';

import {LoginWrapper} from '@semantic/ui/src/component/LoginWrapper';
import {EntityBrowser} from '@semantic/ui/src/component/EntityBrowser';
import {RoutedEntityPage} from '@semantic/ui/src/component/entity/entity_page';
import {AppRoot} from '@semantic/ui/src/component/AppRoot';


import '@mantine/core/styles.css';

function main() {

const router = createBrowserRouter([
  {
    path: "/",
    element: <LoginWrapper>
    	<AppRoot>
    	  <Outlet />
	</AppRoot>
    </LoginWrapper>,
    children: [
    	{errorElement: <div>error</div>},
    	{index: true, element: <EntityBrowser />},
	{path: 'entity/:id', element: <RoutedEntityPage /> },
	{path: '*', element: <div>404</div>},
    ],
  },
]);

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <RouterProvider router={router} />
  </React.StrictMode>,
);
}


main()
